/*
 * battlecore: C interface of the headless battle core (Rust).
 * Matches src/ffi.rs. Link against battlecore.dll / libbattlecore.so /
 * libbattlecore.a built with `cargo build --release`.
 *
 * The core owns all memory. Pointers returned by bc_ships and bc_events stay
 * valid until the next call that changes the battle (bc_step, bc_run_to_pulse,
 * bc_issue, bc_destroy). Positions are fixed point: divide by BcShipView.fp to
 * get distance units. Angles are 0..65535 for one full turn.
 */
#ifndef BATTLECORE_H
#define BATTLECORE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define BC_OK 0
#define BC_ERR_NULL (-1)
#define BC_ERR_PANIC (-2)
#define BC_ERR_BAD_COMMAND (-3)
/* bc_issue returns -10 - n for a rejected command, n being: */
#define BC_CMD_NO_SUCH_GROUP 0
#define BC_CMD_NOT_YOUR_GROUP 1
#define BC_CMD_GROUP_OUT_OF_ACTION 2
#define BC_CMD_NOT_ENOUGH_POINTS 3
#define BC_CMD_GROUP_RETREATING 4
#define BC_CMD_NOT_IN_RESERVE 5
#define BC_CMD_BAD_TARGET 6
#define BC_CMD_BATTLE_OVER 7
#define BC_CMD_SIGNATURE_NOT_READY 8
#define BC_CMD_IN_GRAVITY_WELL 9  /* no jumping out of a gravity well */
#define BC_CMD_FIXED 10           /* structures take no orders */
#define BC_CMD_CHANGING_LAYER 11

/* Command verbs for bc_issue. */
#define BC_VERB_ATTACK 0        /* a = target group */
#define BC_VERB_FLANK_LEFT 1    /* a = target group */
#define BC_VERB_FLANK_RIGHT 2   /* a = target group */
#define BC_VERB_HOLD 3
#define BC_VERB_ADVANCE 4       /* a, b = point in distance units */
#define BC_VERB_FOCUS_PART 5    /* a = BC_PART_* */
#define BC_VERB_SET_DOCTRINE 6  /* a = 0 line, 1 anvil, 2 hammer */
#define BC_VERB_COMMIT_RESERVE 7
#define BC_VERB_RETREAT 8
#define BC_VERB_SIGNATURE 9     /* each ship uses its class's signature move */
#define BC_VERB_CHANGE_LAYER 10 /* main <-> low layer, takes 4 s */

#define BC_PART_ENGINE 1
#define BC_PART_WEAPONS 2
#define BC_PART_SHIELD_GEN 4
#define BC_PART_BRIDGE 8

#define BC_CLASS_BATTLESHIP 0
#define BC_CLASS_CRUISER 1
#define BC_CLASS_DESTROYER 2
#define BC_CLASS_FLAGSHIP 3
#define BC_CLASS_CARRIER 4
#define BC_CLASS_ARTILLERY 5
#define BC_CLASS_SUPPORT 6
#define BC_CLASS_STATION 7
#define BC_CLASS_BEACON 8
#define BC_CLASS_PYLON 9

#define BC_LAYER_LOW 0
#define BC_LAYER_MAIN 1
#define BC_LAYER_MOVING 0x80

/* Terrain cell kinds (bc_header_json "terrain.cells"). */
#define BC_TERRAIN_EMPTY 0
#define BC_TERRAIN_ASTEROIDS 1
#define BC_TERRAIN_NEBULA 2
#define BC_TERRAIN_GRAVITY 3
#define BC_TERRAIN_DEBRIS 4
#define BC_TERRAIN_PLANET 5
#define BC_TERRAIN_FIELD0 6
#define BC_TERRAIN_FIELD1 7

#define BC_SHIP_ALIVE 0
#define BC_SHIP_DEAD 1
#define BC_SHIP_ESCAPED 2

/* Event kinds (fields a, b, c, d per kind). */
#define BC_EV_FIRE 1                /* a shooter, b target, c bit0 hit, bit1 salvo; d damage type */
#define BC_EV_MISSILE_LAUNCH 2      /* a shooter, b missile id, c target, d 1 if torpedo */
#define BC_EV_MISSILE_INTERCEPTED 3 /* a point-defense ship, b missile id */
#define BC_EV_MISSILE_HIT 4         /* a missile id, b target */
#define BC_EV_SHIP_KILLED 5         /* a ship, b killer (0xFFFFFFFF if none) */
#define BC_EV_PART_DAMAGED 6        /* a ship, c part bit */
#define BC_EV_OVERLOAD 7            /* a ship */
#define BC_EV_GROUP_ROUTED 8        /* a group */
#define BC_EV_GROUP_ESCAPED 9       /* a group, c ships saved */
#define BC_EV_RETREAT_STARTED 10    /* a group */
#define BC_EV_COMMAND 11            /* a group, b side, c command kind */
#define BC_EV_PULSE 12              /* c pulse number */
#define BC_EV_FLAGSHIP_LOST 13      /* a ship, b side */
#define BC_EV_BATTLE_OVER 14        /* c winner (0, 1, -1 draw) */
#define BC_EV_SIGNATURE_CHARGING 15 /* a group, c ticks until it fires */
#define BC_EV_SIGNATURE_FIRED 16    /* a group */
#define BC_EV_WING_LAUNCHED 17      /* a wing, b carrier, c fighters */
#define BC_EV_FIGHTER_DOWN 18       /* a wing, b shooter ship, or wing | 0x80000000 */
#define BC_EV_WING_STRIKE 19        /* a wing, b target ship, c damage */
#define BC_EV_WING_LOST 20          /* a wing */
#define BC_EV_LAYER_CHANGED 21      /* a group, c new layer; d 1 when the move starts */
#define BC_EV_BARRAGE_MARKED 22     /* a strike id, b x (int32 units), c y, d side */
#define BC_EV_BARRAGE_HIT 23        /* a strike id, c ships hit */
#define BC_EV_PHASE 24              /* a group, c 0 approach, 1 engage, 2 disengage, 3 refit */
#define BC_EV_JUMP_IN 25            /* a group, b beacon ship */
#define BC_EV_DEBRIS 26             /* a terrain cell index turned to debris */
#define BC_EV_FIELD_LOST 27         /* a pylon ship, b side */
#define BC_EV_SHIELD_BOOST 28       /* a support ship, c ships boosted */

typedef struct BcBattle BcBattle;

typedef struct BcEvent {
    uint32_t tick;
    uint8_t kind;
    uint8_t d;
    uint16_t pad;
    uint32_t a;
    uint32_t b;
    int32_t c;
} BcEvent;

/* Structure of arrays; every array has `count` entries. */
typedef struct BcShipView {
    uint32_t count;
    const int32_t *x;
    const int32_t *y;
    const uint16_t *heading;
    const uint8_t *ship_class;
    const uint8_t *side;
    const uint16_t *group;
    const int32_t *hull;
    const uint8_t *parts;   /* bits of parts still working */
    const uint8_t *state;   /* BC_SHIP_* */
    const uint32_t *target; /* 0xFFFFFFFF when none */
    const uint8_t *layer;   /* BC_LAYER_*, | BC_LAYER_MOVING */
    int32_t fp;             /* fixed-point steps per distance unit */
} BcShipView;

BcBattle *bc_create_demo(uint64_t seed, uint32_t scale);
void bc_destroy(BcBattle *b);
void bc_set_threads(BcBattle *b, uint32_t threads);
/* ai = 0: the side is player-controlled; 1: the built-in commander plays it. */
void bc_set_ai(BcBattle *b, uint32_t side, uint32_t ai);
/* Advance n steps (20 per second). Returns 1 while running, 0 when over. */
int32_t bc_step(BcBattle *b, uint32_t n);
/* Step to the next pulse (pause point). Returns 1 while running. */
int32_t bc_run_to_pulse(BcBattle *b);
int32_t bc_issue(BcBattle *b, uint32_t side, uint32_t group, uint32_t verb, int32_t a, int32_t bb);
BcShipView bc_ships(const BcBattle *b);
/* Events of the last step. */
const BcEvent *bc_events(const BcBattle *b, uint32_t *count);
uint32_t bc_tick(const BcBattle *b);
/* 0 or 1, -1 draw, -2 still running. */
int32_t bc_winner(const BcBattle *b);
int32_t bc_command_points(const BcBattle *b, uint32_t side);
uint64_t bc_state_hash(const BcBattle *b);

/* JSON views (UTF-8, not NUL-terminated; length from bc_json_len). Used by
 * the browser build; handy for tools. Valid until the next call on b. */
const uint8_t *bc_header_json(BcBattle *b);  /* classes, groups, terrain */
const uint8_t *bc_frame_json(BcBattle *b);   /* state + events since last call */
uint32_t bc_json_len(const BcBattle *b);

#ifdef __cplusplus
}
#endif

#endif /* BATTLECORE_H */
