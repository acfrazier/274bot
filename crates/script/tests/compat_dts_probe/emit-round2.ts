// Round-one review consumers: posted fields stay precise and absent runtime fields stay rejected.
import { LoopingBot } from '*api/bot/Bot.js';
import Tile from '*geometry/Tile.js';
import { SettingsBag } from '*runtime/Settings.js';
import { reader } from '*adapter/ClientAdapter.js';
import { SHOP_DB } from '*data/shopdb.js';
import { DROP_DB } from '*data/dropdb.js';
import { resolveRockIds } from '*data/miningRocks.js';
import { nearestCowLocation } from '*data/cowKillerLocations.js';
import { PERIODIC_BANK_SETTINGS } from '*api/bank/Banking.js';

export class PostedConsumers extends LoopingBot {
  override loop(): void {}
  override onStart(): void {
    this.on('inventory.changed', e => {
      const fields: [number, number, string | null, number, number, number] =
        [e.slot, e.id, e.name, e.count, e.previousId, e.previousCount];
      void fields;
      // @ts-expect-error posted item id is a number
      const wrong: string = e.id;
      void wrong;
    });
    this.on('chat.message', e => {
      const fields: [number, string | undefined, string] = [e.type, e.username, e.text];
      void fields;
      // @ts-expect-error absent usernames are undefined, not null
      const wrong: string | null = e.username;
      void wrong;
    });
    this.on('skill.xp', e => {
      const fields: [number, string, number, number] = [e.skill, e.name, e.xp, e.delta];
      void fields;
      // @ts-expect-error posted XP delta is a number
      const wrong: string = e.delta;
      void wrong;
    });
    this.on('skill.level', e => {
      const fields: [number, string, number, number] = [e.skill, e.name, e.level, e.previous];
      void fields;
      // @ts-expect-error the frozen EventMap level contract is numeric
      const wrong: string = e.level;
      void wrong;
    });
    this.on('varp.changed', e => {
      const fields: [number, number, number] = [e.index, e.value, e.previous];
      void fields;
      // @ts-expect-error varp changes do not contain item names
      void e.name;
    });
    this.on('tick', e => {
      const tick: number = e.tick;
      void tick;
      // @ts-expect-error tick events do not contain item ids
      void e.id;
    });
    const event: string = 'custom.event';
    this.on(event, e => {
      // @ts-expect-error arbitrary string subscriptions have unknown payloads
      void e.id;
    });
    const tile = this.settings.tile('anchor', new Tile(3200, 3200, 0));
    const distance: number = tile.distanceTo(tile);
    void distance;
    const optionalTile: Tile | null = this.settings.tile('anchor');
    void optionalTile;
    const plain = this.settings.tile('anchor', { x: 3200, z: 3200, level: 0 });
    // @ts-expect-error a plain fallback is returned unchanged, not promoted to Tile
    plain.distanceTo(plain);
  }
}

const bag = new SettingsBag({});
const real = bag.tile('anchor', new Tile(3200, 3200, 0));
const bagDistance: number = real.distanceTo(real);
void bagDistance;
const absent: Tile | null = bag.tile('anchor');
void absent;
const fallback = bag.tile('anchor', { x: 3200, z: 3200, level: 0 });
if (fallback) {
  // @ts-expect-error review's plain fallback has no Tile methods
  fallback.distanceTo(fallback);
}
const loc = reader.locs()[0];
const locCoordinates: [number, number, number] = [loc.x, loc.z, loc.level];
void locCoordinates;
// @ts-expect-error shape is FlatBuffer-only, never posted to the JS row
loc.shape.toFixed(0);
// @ts-expect-error angle is FlatBuffer-only, never posted to the JS row
loc.angle.toFixed(0);
const keepers: string[] | undefined = SHOP_DB['fishingshop']?.keepers;
const drops: string[] | undefined = DROP_DB['Cow'];
void keepers; void drops;
// @ts-expect-error keeper names are strings, not ids
const wrongKeepers: number[] = SHOP_DB['fishingshop'].keepers;
// @ts-expect-error the drop table contains item names, not numbers
const wrongDrops: number[] = DROP_DB['Cow'];
void wrongKeepers; void wrongDrops;
export function unsupportedRocks(): void {
  // @ts-expect-error resolveRockIds always throws; it does not yield a usable Set
  resolveRockIds(['Copper']).has(1);
}
const strategy = PERIODIC_BANK_SETTINGS.bankStrategy;
void strategy;
// @ts-expect-error the re-export is a structured settings value, not a number
const invalidSettings: number = PERIODIC_BANK_SETTINGS;
void invalidSettings;
const nearest = nearestCowLocation(new Tile(3200, 3200, 0));
if (nearest !== undefined) {
  // @ts-expect-error resolving the selected location can also return null
  void nearest.name;
}
if (nearest != null) {
  const name: string = nearest.name;
  void name;
}
