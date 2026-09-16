// crates/script/examples/bone_burier_v2.ts
export const apiVersion = 2;
export const SETTINGS = {
  boneName: {
    type: "string",
    default: "Bones",
    label: "Bone name",
    description: "The unnoted inventory and bank item to bury"
  }
};
var phase = "finding-bank";
var pending = null;
var waitingForBankSince = 0;
var activeBone = "";
var buried = 0;
var prayerXp = 0;
var initialPrayerXp = null;
function count(items, name) {
  return items.filter((item) => item.name?.toLowerCase() === name.toLowerCase() && !item.noted).reduce((total, item) => total + item.count, 0);
}
function prayer(api) {
  const row = api.snapshot.stats.find((stat) => stat.name.toLowerCase() === "prayer");
  if (!row) return prayerXp;
  if (initialPrayerXp === null) initialPrayerXp = row.xp;
  prayerXp = Math.max(0, row.xp - (initialPrayerXp ?? row.xp));
  return prayerXp;
}
function paint(api, bone) {
  api.paint.begin({
    accent: "#ffb15b"
  }).title("BoneBurier v2").row("bone", bone).row("phase", phase).row("burials", buried).row("prayer XP", prayer(api)).end();
}
function fail(api, reason) {
  phase = "stopping";
  pending = null;
  api.log(`BoneBurier v2: ${reason}`);
  api.stop(reason);
}
function timedOut(api) {
  if (pending && api.tick - pending.sent > 12) {
    fail(api, `stalled while ${pending.kind}; no observed completion`);
    return true;
  }
  if (!pending && waitingForBankSince && api.tick - waitingForBankSince > 12) {
    fail(api, "bank contents unavailable after waiting");
    return true;
  }
  return false;
}
function supportedStand(api) {
  return api.snapshot.banks.find((stand) => api.snapshot.bank_approaches.some((approach) => approach.can_operate && approach.dest_ok && approach.x === stand.x && approach.z === stand.z && approach.level === stand.level)) ?? null;
}
function observePending(api, bone, invCount) {
  if (!pending) return false;
  const work = pending;
  if (work.kind === "bury" && invCount < work.before) {
    buried += work.before - invCount;
    pending = null;
    phase = "burying";
    return true;
  }
  if (work.kind === "load") {
    if (invCount > work.before) {
      pending = null;
      phase = "loading";
      return true;
    }
    if (api.snapshot.withdraw_load_result_seq > work.seq) {
      if (api.snapshot.withdraw_load_result) {
        pending = null;
        return true;
      }
      fail(api, "withdraw refused or inventory has no free slot");
      return true;
    }
  }
  if (work.kind === "open" && api.snapshot.bank_open) {
    pending = null;
    phase = "opening-bank";
    return true;
  }
  if (work.kind === "close" && !api.snapshot.bank_open) {
    pending = null;
    phase = "finding-bank";
    return true;
  }
  if (work.kind === "walk" && api.snapshot.walk_outcome_seq > work.seq) {
    if (api.snapshot.walk_outcome_failed) {
      fail(api, "bank navigation failed");
    } else {
      pending = null;
      phase = "finding-bank";
    }
    return true;
  }
  return false;
}
export function tick(api) {
  const bone = api.settings.str("boneName", "Bones").trim() || "Bones";
  if (activeBone && activeBone !== bone) {
    pending = null;
    waitingForBankSince = 0;
    phase = "finding-bank";
    api.log(`BoneBurier v2: bone changed to ${bone}`);
  }
  activeBone = bone;
  paint(api, bone);
  if (!api.snapshot.ingame) {
    fail(api, "not in game");
    return;
  }
  const invCount = count(api.snapshot.inv, bone);
  if (timedOut(api) || observePending(api, bone, invCount)) return;
  if (phase === "stopping") return;
  if (invCount > 0) {
    waitingForBankSince = 0;
    if (api.snapshot.bank_open) {
      phase = "opening-bank";
      pending = {
        kind: "close",
        sent: api.tick,
        before: invCount,
        seq: api.snapshot.bank_op_result_seq
      };
      api.request({
        op: "close"
      });
      return;
    }
    phase = "burying";
    pending = {
      kind: "bury",
      sent: api.tick,
      before: invCount,
      seq: 0
    };
    api.request({
      op: "held",
      name: bone,
      action: "Bury"
    });
    return;
  }
  if (api.snapshot.bank_open) {
    if (!api.snapshot.bank_loaded) {
      if (!waitingForBankSince) waitingForBankSince = api.tick;
      phase = "loading";
      return;
    }
    waitingForBankSince = 0;
    const row = api.snapshot.bank.find((item) => item.name?.toLowerCase() === bone.toLowerCase() && !item.noted && item.count > 0);
    if (!row) {
      fail(api, "confirmed loaded current-generation bank exhaustion");
      return;
    }
    phase = "loading";
    pending = {
      kind: "load",
      sent: api.tick,
      before: invCount,
      seq: api.snapshot.withdraw_load_result_seq
    };
    api.request({
      op: "withdraw-load",
      name: bone,
      bank_generation: api.snapshot.bank_generation
    });
    return;
  }
  const stand = supportedStand(api);
  if (!stand) {
    if (!api.snapshot.banks.length || !api.snapshot.bank_approaches.length) {
      fail(api, "no supported nearby bank");
    } else if (!pending) {
      phase = "finding-bank";
      pending = {
        kind: "walk",
        sent: api.tick,
        before: 0,
        seq: api.snapshot.walk_outcome_seq
      };
      api.request({
        op: "walk-nearest-bank"
      });
    }
    return;
  }
  phase = "opening-bank";
  pending = {
    kind: "open",
    sent: api.tick,
    before: 0,
    seq: api.snapshot.bank_op_result_seq
  };
  api.request({
    op: "open-stand",
    x: stand.x,
    z: stand.z,
    level: stand.level,
    kind: stand.kind,
    name: stand.name,
    stand_op: stand.op,
    ...stand.choose ? {
      choose: stand.choose
    } : {}
  });
}

