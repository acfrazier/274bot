// crates/script/examples/gather_quest_v2.ts
export const apiVersion = 2;
export const SETTINGS = {
  logs: {
    type: "number",
    default: 56,
    label: "Logs to gather and drop"
  }
};
var ended = null;
var phase = "start";
export async function tick(api) {
  if (phase === "start") {
    api.gather.run({
      skill: "Woodcutting",
      woodcuttingResources: ["normal"],
      location: "Start",
      radius: 12,
      disposition: "Power"
    }).then((out) => {
      ended = out;
    });
    phase = "gathering";
    return;
  }
  if (phase === "gathering" || phase === "stopping") {
    if (ended) {
      api.log("gather outcome: " + JSON.stringify(ended));
      if (ended.kind !== "done") {
        phase = "done";
        api.stop(`gather: ${ended.kind}: ${ended.reason}`);
        return;
      }
      if (ended.value.end !== "stopped") {
        phase = "done";
        api.stop(`gather: ${ended.value.end}`);
        return;
      }
      api.log(`gathered ${ended.value.counts.yielded}, dropped ${ended.value.counts.dropped}`);
      phase = "quest";
      return;
    }
    const logs = api.settings.num("logs", 56);
    const live = api.snapshot.gather;
    if (phase === "gathering" && live?.status && live.status.dropped >= logs) {
      const stopped = api.gather.stop();
      api.log("gather stop: " + JSON.stringify(stopped));
      if (!stopped.ok) {
        phase = "done";
        api.stop(`gather stop: ${stopped.error}`);
        return;
      }
      phase = "stopping";
    }
    return;
  }
  if (phase === "quest") {
    phase = "done";
    const paths = api.questPaths();
    api.log("quest paths: " + JSON.stringify(paths));
    if (!paths.ok) {
      api.stop(`quest paths: ${paths.error}`);
      return;
    }
    const cook = paths.value.rows.find((row) => row.id === "cook");
    if (!cook) {
      api.stop("quest paths: cook missing");
      return;
    }
    const out = await api.questProgress({ quest: "cook" });
    api.log("quest progress: " + JSON.stringify(out));
    if (out.kind !== "done" || out.value.end !== "done") {
      api.stop(`questProgress: ${out.kind}`);
      return;
    }
    const row = out.value.row;
    const stage = row.stage.state === "known" ? row.stage.value : `unknown(${row.stage.gap})`;
    api.log(`Cook's Assistant: colour=${row.colour} stage=${stage} complete=${row.complete}`);
    api.stop("done");
  }
}
