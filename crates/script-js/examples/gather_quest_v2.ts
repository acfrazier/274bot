type NativeApi = import('../host-js/index.d.ts').NativeApi;
type GatherOutcome = import('../host-js/index.d.ts').GatherOutcome;
import type { ScriptManifest } from '../host-js/index.d.ts';

/**
 * Native gather-then-quest sample (JS API v2).
 *
 * The script gathers Power-mode normal logs at Start (radius 12) until the live
 * status reports `dropped >= logs` (setting `logs`, default 56: two full 28-slot
 * power-drop cycles), stops the session, then performs one read-only Cook's
 * Assistant progress check. It issues no game actions of its own: gathering,
 * dropping and the journal read are all host-owned.
 */
export const manifest = {
    apiVersion: 2,
    name: 'GatherQuest',
    version: '1',
    author: '274bot',
    description: 'Gathers and drops normal logs, then reads Cook\'s Assistant progress.',
    category: 'Examples',
    tags: ['gather', 'woodcutting', 'quest'],
    settings: {
        logs: {
            type: 'number',
            default: 56,
            label: 'Logs to gather and drop',
        },
    },
    capabilities: [],
} as const satisfies ScriptManifest;

let ended: GatherOutcome | null = null;
let phase: 'start' | 'gathering' | 'stopping' | 'quest' | 'done' = 'start';

export async function tick(api: NativeApi): Promise<void> {
    if (phase === 'start') {
        // One session; the promise is kept, not awaited, so this tick keeps running.
        api.gather.run({
            skill: 'Woodcutting',
            woodcuttingResources: ['normal'],
            location: 'Start',
            radius: 12,
            disposition: 'Power',
        }).then((out) => { ended = out; });
        phase = 'gathering';
        return;
    }
    if (phase === 'gathering' || phase === 'stopping') {
        if (ended) {
            // The session settled: refused / blocked / failed / aborted / stopped.
            api.log('gather outcome: ' + JSON.stringify(ended));
            if (ended.kind !== 'done') {
                phase = 'done';
                api.stop(`gather: ${ended.kind}: ${ended.reason}`);
                return;
            }
            if (ended.value.end !== 'stopped') {
                phase = 'done';
                api.stop(`gather: ${ended.value.end}`);
                return;
            }
            api.log(`gathered ${ended.value.counts.yielded}, dropped ${ended.value.counts.dropped}`);
            phase = 'quest';
            return;
        }
        const logs = api.settings.num('logs', 56);
        const live = api.snapshot.gather;
        if (phase === 'gathering' && live?.status && live.status.dropped >= logs) {
            const stopped = api.gather.stop();
            api.log('gather stop: ' + JSON.stringify(stopped));
            if ('error' in stopped) {
                phase = 'done';
                api.stop(`gather stop: ${stopped.error}`);
                return;
            }
            phase = 'stopping';
        }
        return;
    }
    if (phase === 'quest') {
        phase = 'done';
        const paths = api.questPaths();
        api.log('quest paths: ' + JSON.stringify(paths));
        if ('error' in paths) {
            api.stop(`quest paths: ${paths.error}`);
            return;
        }
        const cook = paths.value.rows.find((row) => row.id === 'cook');
        if (!cook) {
            api.stop('quest paths: cook missing');
            return;
        }
        const out = await api.questProgress({ quest: 'cook' });  // one await, then the tick returns
        api.log('quest progress: ' + JSON.stringify(out));
        if (out.kind !== 'done' || out.value.end !== 'done') {
            api.stop(`questProgress: ${out.kind}`);
            return;
        }
        const row = out.value.row;
        const stage = row.stage.state === 'known' ? row.stage.value : `unknown(${row.stage.gap})`;
        api.log(`Cook's Assistant: colour=${row.colour} stage=${stage} complete=${row.complete}`);
        api.stop('done');
    }
}
