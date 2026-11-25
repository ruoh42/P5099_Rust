#include <windows.h>

#include <cwchar>

#include "p5099_xun_api.h"

int wmain(int argc, wchar_t** argv) {
    if (argc != 3) {
        std::fwprintf(stderr, L"usage: p5099_xun_attach_smoke <p5099-xun.dll> <P5099.exe>\n");
        return 2;
    }
    HMODULE module = LoadLibraryW(argv[1]);
    if (module == nullptr) {
        std::fwprintf(stderr, L"LoadLibrary failed: %lu\n", GetLastError());
        return 3;
    }
    using GetAbiVersion = uint32_t(P5099_XUN_CALL*)();
    using Initialize = uint32_t(P5099_XUN_CALL*)(void*);
    using GetStatus = int32_t(P5099_XUN_CALL*)(P5099XunStatusSnapshot*);
    using VerifyExecutable = int32_t(P5099_XUN_CALL*)(const wchar_t*);
    const auto get_abi = reinterpret_cast<GetAbiVersion>(GetProcAddress(module, "P5099XunGetAbiVersion"));
    const auto initialize = reinterpret_cast<Initialize>(GetProcAddress(module, "P5099XunInitialize"));
    const auto get_status = reinterpret_cast<GetStatus>(GetProcAddress(module, "P5099XunGetStatus"));
    const auto verify = reinterpret_cast<VerifyExecutable>(GetProcAddress(module, "P5099XunVerifyExecutableFile"));
    if (get_abi == nullptr || initialize == nullptr || get_status == nullptr || verify == nullptr
        || get_abi() != P5099_XUN_ABI_VERSION) {
        std::fwprintf(stderr, L"attachable sidecar exports are missing\n");
        FreeLibrary(module);
        return 4;
    }
    if (verify(argv[2]) != 1 || verify(argv[0]) != 0) {
        std::fwprintf(stderr, L"exact executable verification failed\n");
        FreeLibrary(module);
        return 5;
    }
    const uint32_t initialized_status = initialize(nullptr);
    P5099XunStatusSnapshot snapshot{};
    snapshot.size = sizeof(snapshot);
    if (get_status(&snapshot) != 1
        || initialized_status != P5099_XUN_STATUS_UNSUPPORTED_PROCESS
        || snapshot.status != P5099_XUN_STATUS_UNSUPPORTED_PROCESS
        || (snapshot.flags & (P5099_XUN_FLAG_PROXY_READY | P5099_XUN_FLAG_HOOKS_INSTALLED)) != 0) {
        std::fwprintf(
            stderr,
            L"unexpected attach smoke status=%lu snapshot=%lu flags=0x%08lX\n",
            initialized_status,
            snapshot.status,
            snapshot.flags);
        FreeLibrary(module);
        return 6;
    }
    std::wprintf(
        L"abi=%lu status=%lu flags=0x%08lX exact_file=1 attach_entry=ok\n",
        snapshot.abi_version,
        snapshot.status,
        snapshot.flags);
    FreeLibrary(module);
    return 0;
}
