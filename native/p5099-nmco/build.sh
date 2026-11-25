#!/bin/sh
set -eu
cd "$(dirname "$0")"
mkdir -p out
"${CXX:-i686-w64-mingw32-g++}" -std=c++20 -O2 -Wall -Wextra -shared \
    -static -static-libgcc -static-libstdc++ \
    -DWIN32_LEAN_AND_MEAN -D_WIN32_WINNT=0x0601 \
    -o out/nmcogame.dll src/nmco.cpp nmcogame.def \
    -lws2_32 -Wl,--enable-stdcall-fixup
echo "built $(pwd)/out/nmcogame.dll"
