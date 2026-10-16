import { describe, expect, test } from 'bun:test';
import { joinItemDb, parseItemDefs, parseObjPack } from '../../tools/items/parse.js';

const SCIMITAR = `
[rune_scimitar]
name=Rune scimitar
desc=A vicious curved sword.
cost=25600
iop2=Wield
wearpos=righthand
category=weapon_slash
`;

const TWO_HANDER = `
[rune_2h_sword]
name=Rune 2h sword
cost=51200
wearpos=righthand
wearpos2=lefthand
members=yes
`;

const LOBSTER = `
[lobster]
name=Lobster
cost=150
iop1=Eat
`;

const NOT_GEAR = `
[coins]
name=Coins
cost=1
stackable=yes
`;

describe('parseItemDefs', () => {
    test('reads the slot off wearpos', () => {
        expect(parseItemDefs(SCIMITAR).rune_scimitar).toEqual({
            name: 'Rune scimitar',
            slot: 'righthand',
            twoHanded: false,
            consumable: undefined,
            cost: 25600,
            members: false
        });
    });

    test('a righthand item that also claims lefthand is two-handed', () => {
        expect(parseItemDefs(TWO_HANDER).rune_2h_sword!.twoHanded).toBe(true);
    });

    test('an Eat op makes a consumable with no slot', () => {
        const lobster = parseItemDefs(LOBSTER).lobster!;
        expect(lobster.slot).toBeUndefined();
        expect(lobster.consumable).toBe('eat');
    });

    test('skips objects that are neither equippable nor consumable', () => {
        expect(parseItemDefs(NOT_GEAR).coins).toBeUndefined();
    });
});

describe('parseObjPack', () => {
    test('maps debugname to id', () => {
        expect(parseObjPack('1333=rune_scimitar\n379=lobster\n').get('rune_scimitar')).toBe(1333);
    });
});

describe('joinItemDb', () => {
    test('joins ids onto parsed objects and sorts by name', () => {
        const objs = { ...parseItemDefs(SCIMITAR), ...parseItemDefs(LOBSTER) };
        const db = joinItemDb(objs, parseObjPack('1333=rune_scimitar\n379=lobster\n'));
        expect(db.map(r => r.name)).toEqual(['Lobster', 'Rune scimitar']);
        expect(db[1]).toEqual({
            obj: 'rune_scimitar',
            id: 1333,
            name: 'Rune scimitar',
            slot: 'righthand',
            cost: 25600,
            members: false
        });
    });

    test('drops objects with no id in the pack', () => {
        const db = joinItemDb(parseItemDefs(SCIMITAR), new Map());
        expect(db).toEqual([]);
    });
});
