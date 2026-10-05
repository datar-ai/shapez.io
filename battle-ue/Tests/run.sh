#!/bin/sh
# Builds the battle core and checks the C++ bridge against it (no Unreal needed).
set -e
here=$(cd "$(dirname "$0")" && pwd)
core="$here/../../battle-core"
(cd "$core" && cargo build --release --lib)
out="$here/../Intermediate/bridge_test"
mkdir -p "$(dirname "$out")"
${CXX:-c++} -std=c++17 -O2 -Wall -Wextra -Wshadow -Werror \
  -I"$core/include" -I"$here/../Source/VelaBattle/Public" \
  "$here/bridge_test.cpp" "$here/../Source/VelaBattle/Private/BattleBridge.cpp" \
  "$core/target/release/libbattlecore.a" -lpthread -ldl -lm -o "$out"
"$out"
