
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>

#include <algorithm>
#include <condition_variable>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <deque>
#include <map>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

namespace {

constexpr uint32_t kPlainMagic = 0x001FCA34;
constexpr uint32_t kPlainTrailer = 0x008A119E;
constexpr uint32_t kScrambleMagic = 0x0A98C32A;
constexpr uint32_t kBridgeProtocol = 2;
constexpr uint8_t kHello = 1, kCall = 2, kReply = 3, kEvent = 4;
constexpr uint32_t kRegisterCallback = 0x1002;
constexpr uint32_t kMaxFrame = 0x100000 + 64;
constexpr DWORD kCallTimeoutMs = 10000;

HMODULE g_module = nullptr;
std::mutex g_log_mutex;
FILE* g_log = nullptr;

std::wstring module_directory() {
    wchar_t path[MAX_PATH] = {};
    GetModuleFileNameW(g_module, path, MAX_PATH);
    std::wstring directory(path);
    size_t slash = directory.find_last_of(L"\\/");
    return slash == std::wstring::npos ? L"." : directory.substr(0, slash);
}

void log_line(const char* format, ...) {
    std::lock_guard<std::mutex> lock(g_log_mutex);
    if (!g_log) {
        g_log = _wfopen((module_directory() + L"\\p5099-nmco.log").c_str(), L"a");
        if (!g_log) return;
    }
    SYSTEMTIME now;
    GetLocalTime(&now);
    fprintf(g_log, "%02d:%02d:%02d.%03d ", now.wHour, now.wMinute, now.wSecond, now.wMilliseconds);
    va_list args;
    va_start(args, format);
    vfprintf(g_log, format, args);
    va_end(args);
    fputc('\n', g_log);
    fflush(g_log);
}

struct Session {
    std::wstring server = L"127.0.0.1";
    uint16_t port = 0;
    std::wstring nickname;
    std::wstring ticket;
};

Session read_session() {
    Session session;
    std::wstring ini = module_directory() + L"\\p5099-xun-session.ini";
    wchar_t buffer[256] = {};
    GetPrivateProfileStringW(L"session", L"server", L"127.0.0.1", buffer, 256, ini.c_str());
    session.server = buffer;
    GetPrivateProfileStringW(L"session", L"nickname", L"", buffer, 256, ini.c_str());
    session.nickname = buffer;
    GetPrivateProfileStringW(L"session", L"ticket", L"", buffer, 256, ini.c_str());
    session.ticket = buffer;
    UINT nmco_port = GetPrivateProfileIntW(L"session", L"nmco_port", 0, ini.c_str());
    if (nmco_port == 0) {
        UINT sidecar = GetPrivateProfileIntW(L"session", L"port", 39314, ini.c_str());
        nmco_port = sidecar + 1;
    }
    session.port = static_cast<uint16_t>(nmco_port);
    return session;
}

void put_u32(std::vector<uint8_t>& out, uint32_t value) {
    for (int i = 0; i < 4; ++i) out.push_back(static_cast<uint8_t>(value >> (8 * i)));
}

uint32_t get_u32(const uint8_t* bytes) {
    return bytes[0] | bytes[1] << 8 | bytes[2] << 16 | static_cast<uint32_t>(bytes[3]) << 24;
}

void put_compact(std::vector<uint8_t>& out, uint32_t value) {
    if (value < 0x40) {
        out.push_back(static_cast<uint8_t>(value << 2));
    } else if (value < 0x4000) {
        uint16_t encoded = static_cast<uint16_t>(value << 2 | 1);
        out.push_back(encoded & 0xFF);
        out.push_back(encoded >> 8);
    } else {
        put_u32(out, value << 2 | 2);
    }
}

std::vector<uint8_t> frame_object(const uint8_t* frame, size_t frame_length) {
    if (frame_length < 12) return {};
    uint32_t magic = get_u32(frame);
    size_t n = get_u32(frame + 4);
    if (12 + n > frame_length) return {};
    const uint8_t* payload = frame + 8;
    if (magic == kPlainMagic) return std::vector<uint8_t>(payload, payload + n);
    if (magic != kScrambleMagic || n < 10) return {};
    uint8_t sum = 0, x = 0;
    for (size_t i = 0; i < n - 2; ++i) {
        sum = static_cast<uint8_t>(sum + payload[i]);
        x ^= payload[i];
    }
    uint16_t check = static_cast<uint16_t>(static_cast<uint8_t>(~sum) << 1 | x);
    if ((payload[n - 2] | payload[n - 1] << 8) != check) return {};
    std::vector<uint8_t> reversed(payload, payload + n - 2);
    std::reverse(reversed.begin(), reversed.end());
    uint8_t key = reversed[3];
    std::vector<uint8_t> plain;
    uint8_t previous = 0;
    for (size_t i = 8; i < reversed.size(); ++i) {
        plain.push_back(reversed[i] ^ (previous ^ key));
        previous = reversed[i];
    }
    return plain;
}

std::vector<uint8_t> plain_frame(const uint8_t* object, size_t length) {
    std::vector<uint8_t> frame;
    frame.reserve(length + 12);
    put_u32(frame, kPlainMagic);
    put_u32(frame, static_cast<uint32_t>(length));
    frame.insert(frame.end(), object, object + length);
    put_u32(frame, kPlainTrailer);
    return frame;
}

class Bridge {
public:
    bool call(uint32_t code, const uint8_t* frame, size_t frame_length, std::vector<uint8_t>& reply) {
        std::unique_lock<std::mutex> lock(mutex_);
        if (!ensure_connected(lock)) return false;
        uint32_t sequence = ++next_sequence_;
        std::vector<uint8_t> body;
        body.push_back(kCall);
        put_u32(body, sequence);
        put_u32(body, code);
        body.insert(body.end(), frame, frame + frame_length);
        if (!send_message(body)) {
            drop_connection();
            return false;
        }
        bool arrived = replies_cv_.wait_for(lock, std::chrono::milliseconds(kCallTimeoutMs), [&] {
            return replies_.count(sequence) != 0 || socket_ == INVALID_SOCKET;
        });
        auto found = replies_.find(sequence);
        if (!arrived || found == replies_.end()) {
            log_line("call 0x%04X timed out or lost its connection", code);
            return false;
        }
        bool ok = found->second.first == 1;
        reply = std::move(found->second.second);
        replies_.erase(found);
        return ok;
    }

    void set_callback(HWND window, UINT message) {
        std::lock_guard<std::mutex> lock(events_mutex_);
        window_ = window;
        message_ = message;
        log_line("events go to window %p message 0x%X", window, message);
        events_cv_.notify_all();
    }

private:
    bool ensure_connected(std::unique_lock<std::mutex>& lock) {
        if (socket_ != INVALID_SOCKET) return true;
        DWORD now = GetTickCount();
        if (last_attempt_ != 0 && now - last_attempt_ < 3000) return false;
        last_attempt_ = now;

        static bool winsock = [] {
            WSADATA data;
            return WSAStartup(MAKEWORD(2, 2), &data) == 0;
        }();
        if (!winsock) return false;
        Session session = read_session();
        char host[256] = {};
        WideCharToMultiByte(CP_UTF8, 0, session.server.c_str(), -1, host, sizeof(host), nullptr, nullptr);
        char port[16];
        snprintf(port, sizeof(port), "%u", session.port);
        addrinfo hints = {};
        hints.ai_family = AF_INET;
        hints.ai_socktype = SOCK_STREAM;
        addrinfo* resolved = nullptr;
        if (getaddrinfo(host, port, &hints, &resolved) != 0 || !resolved) {
            log_line("cannot resolve %s:%s", host, port);
            return false;
        }
        SOCKET connection = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
        int result = connect(connection, resolved->ai_addr, static_cast<int>(resolved->ai_addrlen));
        freeaddrinfo(resolved);
        if (result != 0) {
            log_line("cannot connect to %s:%s (%d)", host, port, WSAGetLastError());
            closesocket(connection);
            return false;
        }
        BOOL no_delay = TRUE;
        setsockopt(connection, IPPROTO_TCP, TCP_NODELAY, reinterpret_cast<const char*>(&no_delay), sizeof(no_delay));
        socket_ = connection;

        std::vector<uint8_t> hello;
        hello.push_back(kHello);
        put_u32(hello, kBridgeProtocol);
        auto put_wstr = [&hello](const std::wstring& text) {
            put_compact(hello, static_cast<uint32_t>(text.size()));
            for (wchar_t unit : text) {
                hello.push_back(static_cast<uint8_t>(unit & 0xFF));
                hello.push_back(static_cast<uint8_t>(unit >> 8));
            }
        };
        put_wstr(session.nickname);
        if (!session.ticket.empty()) put_wstr(session.ticket);
        if (!send_message(hello)) {
            drop_connection();
            return false;
        }
        log_line("connected to %s:%s", host, port);
        if (!reader_started_) {
            reader_started_ = true;
            std::thread([this] { read_loop(); }).detach();
            std::thread([this] { dispatch_loop(); }).detach();
        }
        reader_cv_.notify_all();
        (void)lock;
        return true;
    }

    bool send_message(const std::vector<uint8_t>& body) {
        std::vector<uint8_t> packet;
        put_u32(packet, static_cast<uint32_t>(body.size()));
        packet.insert(packet.end(), body.begin(), body.end());
        size_t sent = 0;
        while (sent < packet.size()) {
            int n = send(socket_, reinterpret_cast<const char*>(packet.data() + sent),
                         static_cast<int>(packet.size() - sent), 0);
            if (n <= 0) return false;
            sent += static_cast<size_t>(n);
        }
        return true;
    }

    void drop_connection() {
        if (socket_ != INVALID_SOCKET) {
            closesocket(socket_);
            socket_ = INVALID_SOCKET;
            log_line("disconnected");
        }
        replies_cv_.notify_all();
    }

    static bool read_exact(SOCKET connection, uint8_t* out, size_t length) {
        size_t got = 0;
        while (got < length) {
            int n = recv(connection, reinterpret_cast<char*>(out + got), static_cast<int>(length - got), 0);
            if (n <= 0) return false;
            got += static_cast<size_t>(n);
        }
        return true;
    }

    void read_loop() {
        for (;;) {
            SOCKET connection;
            {
                std::unique_lock<std::mutex> lock(mutex_);
                reader_cv_.wait(lock, [&] { return socket_ != INVALID_SOCKET; });
                connection = socket_;
            }
            uint8_t prefix[4];
            std::vector<uint8_t> body;
            bool ok = read_exact(connection, prefix, 4);
            uint32_t length = ok ? get_u32(prefix) : 0;
            if (ok && (length < 5 || length > kMaxFrame)) ok = false;
            if (ok) {
                body.resize(length);
                ok = read_exact(connection, body.data(), length);
            }
            std::unique_lock<std::mutex> lock(mutex_);
            if (!ok) {
                if (socket_ == connection) drop_connection();
                continue;
            }
            if (body[0] == kReply && body.size() >= 9) {
                uint32_t sequence = get_u32(&body[1]);
                uint32_t result = get_u32(&body[5]);
                replies_[sequence] = {result, std::vector<uint8_t>(body.begin() + 9, body.end())};
                replies_cv_.notify_all();
            } else if (body[0] == kEvent && body.size() >= 5) {
                uint32_t code = get_u32(&body[1]);
                std::lock_guard<std::mutex> events_lock(events_mutex_);
                events_.push_back({code, plain_frame(body.data() + 5, body.size() - 5)});
                events_cv_.notify_all();
            }
        }
    }

    void dispatch_loop() {
        for (;;) {
            std::pair<uint32_t, std::vector<uint8_t>> event;
            HWND window;
            UINT message;
            {
                std::unique_lock<std::mutex> lock(events_mutex_);
                events_cv_.wait(lock, [&] { return !events_.empty() && window_ != nullptr; });
                event = std::move(events_.front());
                events_.pop_front();
                window = window_;
                message = message_;
            }
            log_line("event 0x%04X (%u bytes)", event.first, static_cast<unsigned>(event.second.size()));
            SendMessageW(window, message, event.first, reinterpret_cast<LPARAM>(event.second.data()));
        }
    }

    std::mutex mutex_;
    std::condition_variable replies_cv_;
    std::condition_variable reader_cv_;
    std::map<uint32_t, std::pair<uint32_t, std::vector<uint8_t>>> replies_;
    SOCKET socket_ = INVALID_SOCKET;
    uint32_t next_sequence_ = 0;
    DWORD last_attempt_ = 0;
    bool reader_started_ = false;

    std::mutex events_mutex_;
    std::condition_variable events_cv_;
    std::deque<std::pair<uint32_t, std::vector<uint8_t>>> events_;
    HWND window_ = nullptr;
    UINT message_ = 0;
};

Bridge& bridge() {
    static Bridge* instance = new Bridge();
    return *instance;
}

}

extern "C" {

__declspec(dllexport) int __cdecl NMCO_CallNMFunction(uint32_t code, const uint8_t* frame, uint8_t codec,
                                                        uint8_t** out_frame, uint32_t* out_capacity) {
    if (!frame || !out_frame) return 0;
    uint32_t payload = get_u32(frame + 4);
    if (payload > kMaxFrame) return 0;
    size_t frame_length = 12 + payload;
    if (code == kRegisterCallback) {
        std::vector<uint8_t> object = frame_object(frame, frame_length);
        if (object.size() >= 16) {
            const uint8_t* tail = object.data() + object.size() - 8;
            bridge().set_callback(reinterpret_cast<HWND>(static_cast<uintptr_t>(get_u32(tail))), get_u32(tail + 4));
        }
    }
    std::vector<uint8_t> reply;
    bool ok = bridge().call(code, frame, frame_length, reply);
    log_line("call 0x%04X codec %u in %u -> %s %u", code, codec, static_cast<unsigned>(frame_length),
             ok ? "ok" : "fail", static_cast<unsigned>(reply.size()));
    if (!ok) return 0;
    std::vector<uint8_t> framed = plain_frame(reply.data(), reply.size());
    uint8_t* copy = static_cast<uint8_t*>(HeapAlloc(GetProcessHeap(), 0, framed.size()));
    if (!copy) return 0;
    std::memcpy(copy, framed.data(), framed.size());
    *out_frame = copy;
    if (out_capacity) *out_capacity = static_cast<uint32_t>(framed.size());
    return 1;
}

__declspec(dllexport) void __cdecl NMCO_MemoryFree(void* frame) {
    if (frame) HeapFree(GetProcessHeap(), 0, frame);
}

__declspec(dllexport) int __cdecl NMCO_SetPatchOption(int) { return 1; }
__declspec(dllexport) int __cdecl NMCO_SetLocaleAndRegion(int, int) { return 1; }
__declspec(dllexport) int __cdecl NMCO_SetVersionFileUrlA(const char*) { return 1; }
__declspec(dllexport) int __cdecl NMCO_SetVersionFileUrlW(const wchar_t*) { return 1; }

BOOL WINAPI DllMain(HINSTANCE instance, DWORD reason, LPVOID) {
    if (reason == DLL_PROCESS_ATTACH) {
        g_module = instance;
        DisableThreadLibraryCalls(instance);
        log_line("p5099 nmco bridge loaded");
    }
    return TRUE;
}

}
