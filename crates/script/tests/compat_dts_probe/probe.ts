import { Bank, Game, Inventory, Tile } from '@rs2b0t/api';
import { nearestBankReachable } from '*api/bank/BankLocations.js';
import { Npc } from '*api/npcs/Npcs.js';
import { levelProgress } from '*paint/levelProgress.js';

const okIngame: boolean = Game.ingame();
void okIngame;
void Game.openSideTab(3);
const okCount: number = Inventory.count('Coins');
void okCount;
const okDist: number = new Tile(1, 2).distanceTo({ x: 1, z: 2, level: 0 });
void okDist;
const npcTileDist: number = new Npc({}).tile().distanceTo({ x: 1, z: 2, level: 0 });
void npcTileDist;
Bank.depositAllMatching((name: string, id: number) => name.length > 0 && id >= 0);
const progress = levelProgress(1, 0);
const okLevel: number = progress.level;
const okFrac: number = progress.fraction;
const okRem: number = progress.remaining;
void okLevel;
void okFrac;
void okRem;

async function reachable(): Promise<void> {
  const bank = await nearestBankReachable({ x: 1, z: 2, level: 0 }, null);
  const n: string | undefined = bank?.name;
  const d: number | undefined = bank?.tile?.distanceTo({ x: 1, z: 2, level: 0 });
  const accessName: string | undefined = bank?.access?.name;
  void n;
  void d;
  void accessName;
}
void reachable;

// @ts-expect-error Game.ingame returns boolean, not string
const badGameResult: string = Game.ingame();
void badGameResult;
// @ts-expect-error openSideTab takes a tab number
Game.openSideTab('not-a-tab-number');
