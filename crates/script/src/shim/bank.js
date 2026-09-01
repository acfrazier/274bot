// Our Bank module: deposit / withdraw helpers over the host-posted
// snapshot blob. `bank_side` rows are the open bank's deposit window
// (one `{name, count}` per item); `bank` rows are the withdraw list.
// Every act queues a request on `__rs2b0t_host.interact`
// (`{op:'deposit', name}` / `{op:'withdraw', name, action}` /
// `{op:'close'}`); the isolate thread forwards the queue to the host and
// host-play resolves names through the 274 ObjNames table and dispatches
// the op through the Driver. A name the table does not know never matches
// a posted row (the host posts `name: null` for it), so a request for it
// is never queued. Missing members throw `not v1` — never a fake value.
const host = () => globalThis.__rs2b0t_host || {};
const notV1 = (name) => new Error('not v1: ' + name);
const snap = () => host().snapshot || {};
const queue = (req) => {
    const h = host();
    h.interact = h.interact || [];
    h.interact.push(req);
};

export const Bank = new Proxy(
    {
        isOpen() {
            return snap().bank_open === true;
        },
        // The withdraw list has actually been decoded (a bank whose list
        // has not filled reads empty, which is not proof of an empty bank).
        loaded() {
            return snap().bank_loaded === true;
        },
        ready() {
            return Bank.isOpen() && Bank.loaded();
        },
        items() {
            return snap().bank || [];
        },
        count(name) {
            const wanted = String(name).toLowerCase();
            return (snap().bank || [])
                .filter(
                    (row) =>
                        row && typeof row.name === 'string' && row.name.toLowerCase() === wanted,
                )
                .reduce((sum, row) => sum + (typeof row.count === 'number' ? row.count : 0), 0);
        },
        // Deposit-all the named bank-side item. The deposit op is always
        // the row's "all" op (`Deposit-1/5/10/All`), the only op worth
        // sending here; the specific-op arm is not v1.
        deposit(name) {
            const wanted = String(name).toLowerCase();
            for (const row of snap().bank_side || []) {
                if (row && typeof row.name === 'string' && row.name.toLowerCase() === wanted) {
                    queue({ op: 'deposit', name: row.name });
                }
            }
        },
        depositInventory() {
            Bank.depositAllMatching(() => true);
        },
        depositAllMatching(predicate) {
            if (typeof predicate !== 'function') {
                throw new Error('not v1: Bank.depositAllMatching requires a function');
            }
            for (const row of snap().bank_side || []) {
                if (row && typeof row.name === 'string' && predicate(row.name)) {
                    queue({ op: 'deposit', name: row.name });
                }
            }
        },
        depositAllExcept(keep) {
            const kept = new Set(Array.from(keep || []).map((k) => String(k).toLowerCase()));
            Bank.depositAllMatching((name) => !kept.has(name.toLowerCase()));
        },
        // Withdraw by name + op: an action label string is used verbatim
        // (`Withdraw All` / `Withdraw 10` / `Withdraw 1`, or `'all'` for
        // Withdraw All); a number maps to the brief's op set — all when it
        // would cover the row's whole count (and is 10+), else 10, else 1.
        withdraw(name, amount) {
            const wanted = String(name).toLowerCase();
            let action;
            if (typeof amount === 'string') {
                action = amount.toLowerCase() === 'all' ? 'Withdraw All' : amount;
            } else {
                const n = Number(amount);
                const row = (snap().bank || []).find(
                    (r) => r && typeof r.name === 'string' && r.name.toLowerCase() === wanted,
                );
                const count = row && typeof row.count === 'number' ? row.count : 0;
                action = n >= 10 && n >= count ? 'Withdraw All' : n >= 10 ? 'Withdraw 10' : 'Withdraw 1';
            }
            queue({ op: 'withdraw', name: String(name), action });
        },
        close() {
            queue({ op: 'close' });
        },
    },
    {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notV1('Bank.' + String(prop));
        },
    },
);
