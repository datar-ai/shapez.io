// Thin C++ layer over the battle core's C API (battle-core/include/battlecore.h).
//
// Plain C++17 with no Unreal types, so it also builds and runs on its own
// (battle-ue/Tests/bridge_test.cpp checks it against the real core on Linux).
// The Unreal side (ABattleManager) owns one vela::Battle, steps it on a worker
// thread and turns each Snapshot into instances and effects.
//
// Not thread-safe: one thread at a time may call into a Battle.
#pragma once

#include <cstdint>
#include <string>
#include <vector>

#include "battlecore.h"

namespace vela {

// Ship facts that never change during a battle.
struct ShipInfo {
    uint8_t ship_class = 0; // BC_CLASS_*
    uint8_t side = 0;
    uint16_t group = 0;
};

// Per-ship state of one step, in distance units.
struct ShipState {
    float x = 0, y = 0;
    uint16_t heading = 0; // 0..65535 for a full turn
    uint8_t state = BC_SHIP_DEAD;
    uint8_t layer = BC_LAYER_MAIN; // | BC_LAYER_MOVING while climbing or diving
    uint32_t target = 0xFFFFFFFFu;
    BcShipFx fx{};
};

// Everything the renderer needs from one step.
struct Snapshot {
    uint32_t tick = 0;
    int32_t winner = -2; // 0, 1, -1 draw, -2 running
    int32_t command_points[2] = {0, 0};
    std::vector<ShipState> ships;
    std::vector<BcMissile> missiles;
    std::vector<BcWing> wings;
    std::vector<BcGroup> groups;
    std::vector<BcStrike> strikes;
    std::vector<BcEvent> events; // every event of this step, misses included
    // Filled only when the terrain changed during this step (debris, a pylon
    // falling); empty otherwise.
    std::vector<uint8_t> terrain;

    bool over() const { return winner != -2; }
    bool at_pulse() const { return tick % BC_PULSE_TICKS == 0; }
    uint32_t pulse() const { return tick / BC_PULSE_TICKS; }
};

class Battle {
public:
    Battle() = default;
    ~Battle();
    Battle(const Battle&) = delete;
    Battle& operator=(const Battle&) = delete;

    // Creates the demo battle and runs the deployment step (tick 0, both
    // commanders get their first command points). `blue_ai` hands the
    // player's side to the built-in commander.
    bool create(uint64_t seed, uint32_t scale, bool blue_ai, uint32_t threads);
    bool valid() const { return b_ != nullptr; }

    // Frees the core battle now (the destructor also does it).
    void destroy();

    void set_ai(uint32_t side, bool ai);

    // Advances one step (1/20 s) and writes the new state to `out`.
    // Returns false once the battle is over.
    bool step(Snapshot& out);

    // Writes the current state without stepping (events left empty).
    void capture(Snapshot& out) const;

    // Issues a command for the player. Returns BC_OK or a negative error;
    // see command_error().
    int32_t issue(uint32_t side, uint32_t group, uint32_t verb, int32_t a = 0, int32_t b = 0);

    const std::vector<ShipInfo>& ships() const { return ships_; }
    const std::vector<std::string>& group_names() const { return group_names_; }
    const std::vector<uint8_t>& terrain() const { return terrain_; }
    uint32_t terrain_dim() const { return terrain_dim_; }
    uint32_t terrain_cell() const { return terrain_cell_; }

    std::string report() const;
    uint64_t state_hash() const;

private:
    void fill(Snapshot& out) const;

    BcBattle* b_ = nullptr;
    std::vector<ShipInfo> ships_;
    std::vector<std::string> group_names_;
    std::vector<uint8_t> terrain_;
    uint32_t terrain_dim_ = 0;
    uint32_t terrain_cell_ = 0;
};

// Player-facing text (UTF-8, Traditional Chinese) for a bc_issue result.
const char* command_error(int32_t code);

// --- Conversions shared by the renderer -----------------------------------

// Sim angle (0..65535) to radians, counter-clockwise from +x as the core uses it.
float heading_radians(uint16_t heading);

// Shortest-arc blend between two sim angles; t in [0, 1].
uint16_t lerp_heading(uint16_t from, uint16_t to, float t);

// Height layer a ship is heading for: the other layer while it climbs or dives.
uint8_t target_layer(uint8_t layer);

// Group nearest to (x, y) among the given side's groups still on the field
// (any side when side < 0), within `max_dist`; -1 when none.
int32_t nearest_group(const Snapshot& s, float x, float y, int side, float max_dist);

} // namespace vela
