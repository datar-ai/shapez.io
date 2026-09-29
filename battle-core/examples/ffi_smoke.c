/* Build: cc examples/ffi_smoke.c -Iinclude target/release/libbattlecore.a -lpthread -ldl -lm */
#include <stdio.h>
#include "battlecore.h"

int main(void) {
    BcBattle *b = bc_create_demo(1, 1);
    bc_set_threads(b, 2);
    int pulses = 0;
    while (bc_run_to_pulse(b) == 1) pulses++;
    BcShipView v = bc_ships(b);
    uint32_t alive = 0;
    for (uint32_t i = 0; i < v.count; i++) alive += v.state[i] == BC_SHIP_ALIVE;
    printf("winner %d after %d pulses, tick %u, %u/%u ships alive, hash %016llx\n",
           bc_winner(b), pulses, bc_tick(b), alive, v.count, (unsigned long long)bc_state_hash(b));
    bc_destroy(b);
    return 0;
}
