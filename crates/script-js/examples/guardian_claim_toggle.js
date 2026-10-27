// Real Load fixture for the seeded Strange box Guardian handoff cell.
// Keep the first observed box script-owned for thirty observed game ticks.
import { Inventory } from "../../api/inventory/Inventory.js";
import { BotHost } from "../../runtime/BotHost.js";
import { EventSignal } from "../../api/execution/EventSignal.js";
import { Paint } from "../../paint/Paint.js";
const STRANGE_BOX_ID = 3062;
const STRANGE_BOX_EVENT = "strange box";
const IGNORE_TICKS = 30;

export default class GuardianClaimToggle extends LoopingBot {
  constructor() {
    super();
    this.firstBoxTick = null;
    this.removeTickListener = null;
    this.resumed = false;
  }

  onStart() {
    this.removeTickListener = BotHost.addTickListener(() => {
      if (this.firstBoxTick === null && Inventory.countById(STRANGE_BOX_ID) > 0) {
        this.firstBoxTick = BotHost.tickCount;
      }
    });
  }

  onStop() {
    if (this.removeTickListener !== null) this.removeTickListener();
  }

  ignoredRandoms() {
    const owns = this.firstBoxTick === null ||
      BotHost.tickCount - this.firstBoxTick < IGNORE_TICKS;
    return owns ? [STRANGE_BOX_EVENT] : [];
  }

  loop() {
    if (
      !this.resumed &&
      this.firstBoxTick !== null &&
      Inventory.countById(STRANGE_BOX_ID) === 0 &&
      !EventSignal.pending()
    ) {
      this.resumed = true;
    }
  }

  onPaint() {
    const boxCount = Inventory.countById(STRANGE_BOX_ID);
    const ignored = this.ignoredRandoms();
    const paint = Paint.begin();
    if (this.firstBoxTick === null && ignored.includes(STRANGE_BOX_EVENT)) {
      paint.row("guardian-box:armed");
    }
    if (boxCount > 0 && ignored.includes(STRANGE_BOX_EVENT)) {
      paint.row("guardian-box:ignored");
    } else if (boxCount > 0 && ignored.length === 0) {
      paint.row("guardian-box:eligible");
    }
    if (this.resumed) {
      paint.row("guardian-box:resumed");
    }
    paint.end();
  }
}
