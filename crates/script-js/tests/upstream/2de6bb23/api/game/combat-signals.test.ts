import { describe, expect, test } from 'bun:test';

import { facingPlayer, PLAYER_FACE_BASE, RETALIATE_VARP, retaliateOnFromVarp } from '#/bot/api/game/Game.js';

describe('auto-retaliate varp', () => {
    test('reads option_nodef', () => {
        expect(RETALIATE_VARP).toBe(172);
    });

    test('is inverted — 0 means ON', () => {
        expect(retaliateOnFromVarp(0)).toBe(true);
        expect(retaliateOnFromVarp(1)).toBe(false);
    });
});

describe('face target encoding', () => {
    test('a player target is slot + 32768', () => {
        expect(PLAYER_FACE_BASE).toBe(32768);
        expect(facingPlayer(PLAYER_FACE_BASE)).toBe(true);
        expect(facingPlayer(PLAYER_FACE_BASE + 2047)).toBe(true);
    });

    test('an npc target or no target is not a player', () => {
        expect(facingPlayer(0)).toBe(false);
        expect(facingPlayer(1234)).toBe(false);
        expect(facingPlayer(-1)).toBe(false);
    });
});
