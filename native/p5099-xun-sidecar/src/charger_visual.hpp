#pragma once

#include <cstdint>

void p5099_xun_configure_charger_visual(std::uintptr_t image_base) noexcept;
void* p5099_xun_create_charger_visual(void* kart) noexcept;
bool p5099_xun_set_charger_visual_active(
    void* effect,
    bool active,
    std::uint32_t now_ms) noexcept;
void p5099_xun_tick_charger_visual(void* effect, std::uint32_t now_ms) noexcept;
