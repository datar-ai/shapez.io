#include "BattleBridge.h"

#include <cmath>

namespace vela {

namespace {

template <typename T, typename F>
void read_all(std::vector<T>& out, F&& fn) {
    uint32_t n = fn(nullptr, 0u);
    out.resize(n);
    if (n > 0) {
        uint32_t again = fn(out.data(), n);
        if (again < n) out.resize(again);
    }
}

} // namespace

Battle::~Battle() {
    destroy();
}

void Battle::destroy() {
    if (b_) bc_destroy(b_);
    b_ = nullptr;
}

bool Battle::create(uint64_t seed, uint32_t scale, bool blue_ai, uint32_t threads) {
    if (b_) bc_destroy(b_);
    b_ = bc_create_demo(seed, scale);
    if (!b_) return false;
    bc_set_threads(b_, threads == 0 ? 1 : threads);
    bc_set_ai(b_, 0, blue_ai ? 1 : 0);
    bc_set_ai(b_, 1, 1);

    BcShipView v = bc_ships(b_);
    ships_.resize(v.count);
    for (uint32_t i = 0; i < v.count; ++i) {
        ships_[i].ship_class = v.ship_class[i];
        ships_[i].side = v.side[i];
        ships_[i].group = v.group[i];
    }
    uint32_t groups = bc_groups(b_, nullptr, 0);
    group_names_.resize(groups);
    for (uint32_t g = 0; g < groups; ++g) {
        uint32_t len = 0;
        const uint8_t* p = bc_group_name(b_, g, &len);
        group_names_[g].assign(reinterpret_cast<const char*>(p), len);
    }
    const uint8_t* cells = bc_terrain(b_, &terrain_dim_, &terrain_cell_);
    terrain_.assign(cells, cells + size_t(terrain_dim_) * terrain_dim_);

    bc_step(b_, 1); // deployment: first command points, nothing moves
    return true;
}

void Battle::set_ai(uint32_t side, bool ai) {
    if (b_) bc_set_ai(b_, side, ai ? 1 : 0);
}

bool Battle::step(Snapshot& out) {
    if (!b_) return false;
    int32_t running = bc_step(b_, 1);
    fill(out);
    uint32_t n = 0;
    const BcEvent* ev = bc_events(b_, &n);
    out.events.assign(ev, ev + n);
    bool terrain_changed = false;
    for (const BcEvent& e : out.events) {
        terrain_changed |= e.kind == BC_EV_DEBRIS || e.kind == BC_EV_FIELD_LOST;
    }
    out.terrain.clear();
    if (terrain_changed) {
        const uint8_t* cells = bc_terrain(b_, nullptr, nullptr);
        terrain_.assign(cells, cells + terrain_.size());
        out.terrain = terrain_;
    }
    return running == 1;
}

void Battle::capture(Snapshot& out) const {
    if (!b_) return;
    fill(out);
    out.events.clear();
    out.terrain.clear();
}

void Battle::fill(Snapshot& out) const {
    out.tick = bc_tick(b_);
    out.winner = bc_winner(b_);
    out.command_points[0] = bc_command_points(b_, 0);
    out.command_points[1] = bc_command_points(b_, 1);

    BcShipView v = bc_ships(b_);
    out.ships.resize(v.count);
    const float inv = 1.0f / float(v.fp);
    std::vector<BcShipFx> fx(v.count);
    bc_ship_fx(b_, fx.data(), v.count);
    for (uint32_t i = 0; i < v.count; ++i) {
        ShipState& s = out.ships[i];
        s.x = float(v.x[i]) * inv;
        s.y = float(v.y[i]) * inv;
        s.heading = v.heading[i];
        s.state = v.state[i];
        s.layer = v.layer[i];
        s.target = v.target[i];
        s.fx = fx[i];
    }
    BcBattle* b = b_;
    read_all(out.missiles, [b](BcMissile* o, uint32_t c) { return bc_missiles(b, o, c); });
    read_all(out.wings, [b](BcWing* o, uint32_t c) { return bc_wings(b, o, c); });
    read_all(out.groups, [b](BcGroup* o, uint32_t c) { return bc_groups(b, o, c); });
    read_all(out.strikes, [b](BcStrike* o, uint32_t c) { return bc_strikes(b, o, c); });
}

int32_t Battle::issue(uint32_t side, uint32_t group, uint32_t verb, int32_t a, int32_t b) {
    if (!b_) return BC_ERR_NULL;
    return bc_issue(b_, side, group, verb, a, b);
}

std::string Battle::report() const {
    if (!b_) return {};
    uint32_t len = 0;
    const uint8_t* p = bc_report_text(b_, &len);
    return p ? std::string(reinterpret_cast<const char*>(p), len) : std::string();
}

uint64_t Battle::state_hash() const {
    return b_ ? bc_state_hash(b_) : 0;
}

const char* command_error(int32_t code) {
    switch (code) {
    case BC_OK: return "";
    case BC_ERR_NULL: return "戰鬥尚未建立";
    case BC_ERR_PANIC: return "模擬核心內部錯誤";
    case BC_ERR_BAD_COMMAND: return "無效的指令";
    default: break;
    }
    switch (-10 - code) {
    case BC_CMD_NO_SUCH_GROUP: return "沒有這個戰鬥群";
    case BC_CMD_NOT_YOUR_GROUP: return "不是你的戰鬥群";
    case BC_CMD_GROUP_OUT_OF_ACTION: return "這個戰鬥群已經退出戰鬥";
    case BC_CMD_NOT_ENOUGH_POINTS: return "指令點不夠";
    case BC_CMD_GROUP_RETREATING: return "這個戰鬥群正在撤離";
    case BC_CMD_NOT_IN_RESERVE: return "只有預備隊能投入";
    case BC_CMD_BAD_TARGET: return "目標無效";
    case BC_CMD_BATTLE_OVER: return "戰鬥已經結束";
    case BC_CMD_SIGNATURE_NOT_READY: return "招牌技還在冷卻";
    case BC_CMD_IN_GRAVITY_WELL: return "在重力井裡不能跳躍撤離";
    case BC_CMD_FIXED: return "建築不接受指令";
    case BC_CMD_CHANGING_LAYER: return "正在換層";
    default: return "指令被拒絕";
    }
}

float heading_radians(uint16_t heading) {
    return float(heading) * (6.28318530718f / 65536.0f);
}

uint16_t lerp_heading(uint16_t from, uint16_t to, float t) {
    int32_t d = int32_t(int16_t(uint16_t(to - from))); // shortest arc, -32768..32767
    return uint16_t(int32_t(from) + int32_t(std::lround(float(d) * t)));
}

uint8_t target_layer(uint8_t layer) {
    uint8_t l = layer & 0x7f;
    if (layer & BC_LAYER_MOVING) return l == BC_LAYER_LOW ? BC_LAYER_MAIN : BC_LAYER_LOW;
    return l;
}

int32_t nearest_group(const Snapshot& s, float x, float y, int side, float max_dist) {
    int32_t best = -1;
    float best_d2 = max_dist * max_dist;
    for (size_t g = 0; g < s.groups.size(); ++g) {
        const BcGroup& gr = s.groups[g];
        if (side >= 0 && gr.side != side) continue;
        if (gr.alive == 0 || gr.status == BC_GROUP_ESCAPED || gr.status == BC_GROUP_DESTROYED) continue;
        float dx = float(gr.cx) - x, dy = float(gr.cy) - y;
        float d2 = dx * dx + dy * dy;
        if (d2 <= best_d2) {
            best_d2 = d2;
            best = int32_t(g);
        }
    }
    return best;
}

} // namespace vela
