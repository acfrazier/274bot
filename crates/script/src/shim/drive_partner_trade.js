// Name-map driveActivePartnerTrade onto a Rust-owned already-open exchange.
// This shim invokes caller callbacks and projects their decisions/metrics.
// It does not sequence offer/confirm, choose trade rows, or treat a close
// as a transfer.
import { notImpl, queue } from '../../shim/_kernel.js';
import { Execution } from '../execution/Execution.js';
import { Trade } from './Trade.js';

function callExchange(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_drive_partner_trade
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('driveActivePartnerTrade');
    }
    return fn(payload);
}

function asNames(value) {
    if (typeof value === 'function') {
        try {
            value = value();
        } catch (err) {
            return [];
        }
    }
    if (!Array.isArray(value)) return [];
    return value.map((name) => String(name ?? '')).filter((name) => name.length > 0);
}

function asMetric(fn) {
    if (typeof fn !== 'function') return null;
    const n = Number(fn());
    return Number.isFinite(n) ? n : null;
}

function project(opts) {
    const their = Trade.theirOffer() || [];
    const match = typeof opts.theirProductMatch === 'function' ? opts.theirProductMatch : () => false;
    return {
        names: asNames(opts.productNamesToOffer),
        metric: asMetric(opts.inventoryMetric),
        myOfferReady: typeof opts.myOfferReady === 'function' ? !!opts.myOfferReady() : null,
        their: their.map((row) => ({
            name: row?.name ?? '',
            count: row?.count ?? 0,
            matched: !!match(row?.name ?? ''),
        })),
    };
}

function applyStatus(opts, step) {
    if (!step || typeof opts.setStatus !== 'function') return;
    const key = step.status;
    if (!key) return;
    const labels = opts.labels || {};
    opts.setStatus(labels[key] || key);
    if (step.log && typeof opts.log === 'function') opts.log(step.log);
}

function dispatchVerb(step) {
    for (const op of step.ops || []) queue(op);
}

function receiverGate(opts, count) {
    if (typeof opts.receiverCanAccept !== 'function') return { ok: true };
    const result = opts.receiverCanAccept(count);
    if (result === true || result?.ok === true) return { ok: true };
    if (result === false) return { ok: false, reason: 'receiver refused' };
    return { ok: false, reason: String(result?.reason || 'receiver refused') };
}

function decide(opts, step) {
    if (!step) return step;
    applyStatus(opts, step);
    if (step.kind === 'need_missing_partner') {
        const action = typeof opts.onMissingPartner === 'function' ? opts.onMissingPartner() : 'wait';
        return decide(opts, callExchange({
            op: 'decision',
            token: step.token,
            missing: action === 'decline' ? 'decline' : 'wait',
            ...project(opts),
        }));
    }
    if (step.kind === 'need_receiver_gate') {
        return decide(opts, callExchange({
            op: 'decision',
            token: step.token,
            receiverGate: receiverGate(opts, step.theirProductCount ?? 0),
            ...project(opts),
        }));
    }
    return step;
}

export async function driveActivePartnerTrade(opts) {
    if (!Trade.active()) return;
    const options = opts || {};
    let current = decide(options, callExchange({
        op: 'begin',
        role: options.role,
        partners: Array.isArray(options.partners) ? options.partners.map((n) => String(n ?? '')) : [],
        verifyGiverPartner: options.verifyGiverPartner === true,
        ...project(options),
    }));
    while (current) {
        if (current.kind === 'notImpl') throw notImpl('driveActivePartnerTrade', current.reason);
        if (current.kind === 'aborted') return;
        if (current.kind === 'complete') {
            if (typeof options.onComplete === 'function') options.onComplete(current.delta ?? 0);
            return;
        }
        if (current.kind === 'declined') {
            if (typeof options.onDecline === 'function') options.onDecline(current.reason || '');
            return;
        }
        if (current.kind === 'ops') dispatchVerb(current);
        else if (current.kind !== 'wait') return;
        const token = current.token;
        let next = null;
        await Execution.delayUntil(() => {
            next = decide(options, callExchange({
                op: 'next',
                token,
                ...project(options),
            }));
            return next?.kind !== 'wait';
        }, 0);
        current = next;
    }
}
