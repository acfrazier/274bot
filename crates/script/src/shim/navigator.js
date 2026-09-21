import { queue, proxy } from '../../shim/_kernel.js';
import { Execution } from '../../api/execution/Execution.js';

function inspectNative(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_inspect(payload);
}

export const Navigator = proxy('Navigator', {
    start() {},
    isReady() {
        return globalThis.__rs2b0t_host?.nav_ready === true;
    },
    async findPath(from, to, opts = {}) {
        const timeoutMs = opts.timeoutMs ?? 20000;
        const token = inspectNative({
            op: 'begin',
            from,
            to,
            opts,
            timeout_ms: timeoutMs,
        });
        if (inspectNative({ op: 'settled', token }) === true) {
            return inspectNative({ op: 'value', token });
        }
        queue({
            op: 'inspect-route',
            x: to.x,
            z: to.z,
            level: to.level ?? 0,
            from_x: from.x,
            from_z: from.z,
            from_level: from.level ?? 0,
            allow_teleports:
                opts.useTeleportCatalog === true || opts.policy?.useTeleports === true,
            allow_wilderness: true,
            allow_bank_fetch: false,
            avoid: opts.avoidZones || [],
            request_id: token,
            inspect_ack_seq: inspectNative({ op: 'ack_seq' }),
        });
        const okWait = await Execution.delayUntil(
            () => inspectNative({ op: 'settled', token }) === true,
            timeoutMs,
        );
        return inspectNative({ op: 'value', token, timed_out: !okWait });
    },
});

export default Navigator;
