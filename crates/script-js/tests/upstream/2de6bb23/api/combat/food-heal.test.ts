import { describe, expect, test } from 'bun:test';
import {
    FOOD_OPTIONS,
    MIN_EAT_HP,
    eatAtHpThreshold,
    foodHealAmount,
    shouldEatFood,
    shouldEatToUseFood
} from '#/bot/api/combat/food.js';

describe('foodHealAmount', () => {
    test('knows common combat foods', () => {
        expect(foodHealAmount('Trout')).toBe(7);
        expect(foodHealAmount('Lobster')).toBe(12);
        expect(foodHealAmount('Swordfish')).toBe(14);
        expect(foodHealAmount('Shark')).toBe(20);
        expect(foodHealAmount('Cake')).toBe(4);
        expect(foodHealAmount('2/3 cake')).toBe(4);
        expect(foodHealAmount('slice of cake')).toBe(4);
    });
});

describe('shouldEatToUseFood (#465)', () => {
    test('eats when a full heal fits without overheal', () => {
        // max 50, lobster 12 → room >= 12 when hp <= 38
        expect(shouldEatToUseFood({ hp: 38, maxHp: 50, heal: 12, foodCount: 2 })).toBe(true);
        expect(shouldEatToUseFood({ hp: 39, maxHp: 50, heal: 12, foodCount: 2 })).toBe(false);
    });

    test('always eats at or below the safety floor when food remains', () => {
        expect(MIN_EAT_HP).toBe(5);
        expect(shouldEatToUseFood({ hp: 5, maxHp: 99, heal: 12, foodCount: 1 })).toBe(true);
        expect(shouldEatToUseFood({ hp: 5, maxHp: 99, heal: 12, foodCount: 0 })).toBe(false);
    });

    test('low-max accounts still honor the floor when food heals more than max', () => {
        expect(eatAtHpThreshold(10, 12)).toBe(5);
        expect(shouldEatToUseFood({ hp: 5, maxHp: 10, heal: 12, foodCount: 1 })).toBe(true);
        expect(shouldEatToUseFood({ hp: 6, maxHp: 10, heal: 12, foodCount: 1 })).toBe(false);
    });

    test('shouldEatFood resolves heal from the food name', () => {
        expect(shouldEatFood('Trout', { hp: 33, maxHp: 40, foodCount: 1 })).toBe(true); // 40-7=33
        expect(shouldEatFood('Trout', { hp: 34, maxHp: 40, foodCount: 1 })).toBe(false);
        // Shark 20 → eat when max−hp ≥ 20
        expect(shouldEatFood('Shark', { hp: 79, maxHp: 99, foodCount: 1 })).toBe(true);
        expect(shouldEatFood('Shark', { hp: 80, maxHp: 99, foodCount: 1 })).toBe(false);
    });
});

describe('FOOD_OPTIONS', () => {
    test('includes Shark so combat scripts (MossGiant, etc.) can select it', () => {
        expect(FOOD_OPTIONS).toContain('Shark');
    });
});
