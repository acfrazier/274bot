import { plannedPotions } from '*api/combat/boostPotions.js';
import { LoopingBot } from '*api/bot/Bot.js';

plannedPotions([{ item: 'Super attack(4)', qty: 5 }]);

class SyncConsumer extends LoopingBot {
  override loop(): void {}
}

void SyncConsumer;
