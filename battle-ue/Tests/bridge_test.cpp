// Checks the C++ bridge (Source/VelaBattle/.../BattleBridge.*) against the real
// battle core, without Unreal. Run: Tests/run.sh (Linux or macOS).
#include <cassert>
#include <cstddef>
#include <cstdio>
#include <cstring>

#include "BattleBridge.h"

// The record layouts the header promises; src/ffi.rs pins the same numbers.
static_assert(sizeof(BcEvent) == 20, "BcEvent");
static_assert(sizeof(BcShipFx) == 4, "BcShipFx");
static_assert(sizeof(BcMissile) == 20 && offsetof(BcMissile, side) == 16, "BcMissile");
static_assert(sizeof(BcWing) == 24 && offsetof(BcWing, side) == 20, "BcWing");
static_assert(sizeof(BcGroup) == 48, "BcGroup");
static_assert(offsetof(BcGroup, alive) == 28 && offsetof(BcGroup, fighters_out) == 36, "BcGroup");
static_assert(offsetof(BcGroup, side) == 40 && offsetof(BcGroup, focus_part) == 47, "BcGroup");
static_assert(sizeof(BcStrike) == 16, "BcStrike");

#define CHECK(cond)                                                              \
    do {                                                                         \
        if (!(cond)) {                                                           \
            std::fprintf(stderr, "%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #cond); \
            return 1;                                                            \
        }                                                                        \
    } while (0)

static int test_auto_battle() {
    vela::Battle b;
    CHECK(b.create(1, 1, /*blue_ai*/ true, 2));
    CHECK(b.ships().size() == 88);
    CHECK(!b.group_names().empty());
    CHECK(b.terrain().size() == size_t(b.terrain_dim()) * b.terrain_dim());

    vela::Snapshot s;
    b.capture(s);
    CHECK(s.tick == 0 && s.at_pulse());

    uint32_t pulses = 0, kills = 0, fires = 0, terrain_updates = 0;
    size_t most_wings = 0, most_missiles = 0;
    bool saw_strike = false;
    while (b.step(s)) {
        if (s.at_pulse()) ++pulses;
        for (const BcEvent& e : s.events) {
            kills += e.kind == BC_EV_SHIP_KILLED;
            fires += e.kind == BC_EV_FIRE;
        }
        terrain_updates += !s.terrain.empty();
        most_wings = s.wings.size() > most_wings ? s.wings.size() : most_wings;
        most_missiles = s.missiles.size() > most_missiles ? s.missiles.size() : most_missiles;
        saw_strike |= !s.strikes.empty();
        CHECK(s.ships.size() == b.ships().size());
        CHECK(s.groups.size() == b.group_names().size());
    }
    CHECK(s.over());
    CHECK(kills > 10 && fires > 1000);
    CHECK(most_wings > 0 && most_missiles > 0);
    std::printf("auto battle: winner %d, tick %u, %u pulses, %u kills, %u shots, "
                "max %zu wings, max %zu missiles, %u terrain updates, strikes %s, hash %016llx\n",
                s.winner, s.tick, pulses, kills, fires, most_wings, most_missiles,
                terrain_updates, saw_strike ? "yes" : "no", (unsigned long long)b.state_hash());

    // Same battle through the raw C API: identical end state.
    BcBattle* raw = bc_create_demo(1, 1);
    bc_set_threads(raw, 1);
    bc_set_ai(raw, 0, 1);
    while (bc_run_to_pulse(raw) == 1) {}
    CHECK(bc_state_hash(raw) == b.state_hash());
    bc_destroy(raw);

    std::string report = b.report();
    CHECK(report.find("結果") != std::string::npos);
    return 0;
}

static int test_player_orders() {
    vela::Battle b;
    CHECK(b.create(7, 1, /*blue_ai*/ false, 1));
    vela::Snapshot s;
    b.capture(s);
    int blue = -1, red = -1, structure = -1;
    for (size_t g = 0; g < s.groups.size(); ++g) {
        const BcGroup& gr = s.groups[g];
        if (gr.fixed) { if (gr.side == 0) structure = int(g); continue; }
        if (gr.status != BC_GROUP_ACTIVE) continue;
        if (gr.side == 0 && blue < 0) blue = int(g);
        if (gr.side == 1 && red < 0) red = int(g);
    }
    CHECK(blue >= 0 && red >= 0 && structure >= 0);

    int32_t r = b.issue(0, red, BC_VERB_HOLD);
    CHECK(r == -10 - BC_CMD_NOT_YOUR_GROUP);
    CHECK(std::strcmp(vela::command_error(r), "不是你的戰鬥群") == 0);
    CHECK(b.issue(0, structure, BC_VERB_HOLD) == -10 - BC_CMD_FIXED);

    CHECK(b.issue(0, blue, BC_VERB_ATTACK, red) == BC_OK);
    CHECK(b.issue(0, blue, BC_VERB_HOLD) == BC_OK);
    CHECK(b.issue(0, blue, BC_VERB_CHANGE_LAYER) == BC_OK);
    // Three points spent.
    CHECK(b.issue(0, blue, BC_VERB_HOLD) == -10 - BC_CMD_NOT_ENOUGH_POINTS);
    b.capture(s);
    CHECK(s.command_points[0] == 0);

    // Runs on to the next pulse, then waits for orders again.
    uint32_t steps = 0;
    bool moving_seen = false, landed_low = false;
    while (b.step(s) && !s.at_pulse()) {
        ++steps;
        moving_seen |= (s.groups[blue].layer & BC_LAYER_MOVING) != 0;
        landed_low |= s.groups[blue].layer == BC_LAYER_LOW;
    }
    CHECK(s.at_pulse() && steps == BC_PULSE_TICKS - 1);
    CHECK(s.command_points[0] == 3);
    CHECK(moving_seen && landed_low);
    for (size_t i = 0; i < s.ships.size(); ++i) {
        if (b.ships()[i].group == blue && s.ships[i].state == BC_SHIP_ALIVE) {
            CHECK(s.ships[i].layer == BC_LAYER_LOW);
        }
    }
    std::printf("player orders: pulse %u reached after %u steps, blue group %d now on the low layer\n",
                s.pulse(), steps + 1, blue);
    return 0;
}

static int test_helpers() {
    CHECK(vela::lerp_heading(65000, 500, 0.5f) == uint16_t(65000 + (1036 / 2)));
    CHECK(vela::lerp_heading(100, 200, 1.0f) == 200);
    CHECK(vela::lerp_heading(200, 100, 0.0f) == 200);
    CHECK(vela::target_layer(BC_LAYER_MAIN) == BC_LAYER_MAIN);
    CHECK(vela::target_layer(BC_LAYER_MAIN | BC_LAYER_MOVING) == BC_LAYER_LOW);
    CHECK(vela::target_layer(BC_LAYER_LOW | BC_LAYER_MOVING) == BC_LAYER_MAIN);

    vela::Snapshot s;
    BcGroup g{};
    g.alive = 3;
    g.cx = 100; g.cy = 0; g.side = 0;
    s.groups.push_back(g);
    g.cx = 1000; g.side = 1;
    s.groups.push_back(g);
    CHECK(vela::nearest_group(s, 900, 0, -1, 500) == 1);
    CHECK(vela::nearest_group(s, 900, 0, 0, 500) == -1);
    CHECK(vela::nearest_group(s, 900, 0, 0, 900) == 0);
    return 0;
}

int main() {
    if (test_helpers() || test_player_orders() || test_auto_battle()) return 1;
    std::printf("bridge tests passed\n");
    return 0;
}
