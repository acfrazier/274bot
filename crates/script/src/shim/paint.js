// Recording Paint: begin returns a frame whose methods append to ScriptPaint
// on the host handle. Unused widgets throw `not impl`. Chrome selection,
// rowsLeft budget and statGrid column policy live in the Rust helper: body
// rows spend the dock budget in one batched crossing instead of one per row.
import { host, notImpl } from '../shim/_kernel.js';

function chrome(op, a, b, c, d) {
    const fn = globalThis.__rs2b0t_paint_chrome;
    if (typeof fn !== 'function') {
        throw notImpl('Paint.' + op);
    }
    return fn(op, a, b, c, d);
}

function asNames(names) {
    if (!names) {
        return [];
    }
    const out = [];
    for (let i = 0; i < names.length; i++) {
        out.push(String(names[i]));
    }
    return out;
}

export const Paint = {
    begin(ctx, opts) {
        const dock = opts && opts.dock != null ? opts.dock : 'chatbox';
        chrome('begin', typeof dock === 'string' ? dock : 'chatbox');
        const rec = {
            title: null,
            accent: (opts && opts.accent) || null,
            lines: [],
            buttons: [],
            selects: {},
            strip: null,
            rail: null,
            footer: null,
            tabBands: [],
        };
        // Vertical spend is a running sum, so the unflushed rows land in the
        // same place as one call per row; only a read has to see them.
        let pendingRows = 0;
        const flushRows = () => {
            if (pendingRows === 0) {
                return;
            }
            chrome('rows', pendingRows);
            pendingRows = 0;
        };
        const frame = {
            title(text) {
                rec.title = String(text);
                chrome('title');
                return frame;
            },
            row(...cols) {
                rec.lines.push(cols.join(' | '));
                pendingRows += 1;
                return frame;
            },
            gap(px) {
                rec.lines.push('');
                chrome('gap', px);
                return frame;
            },
            text(line) {
                rec.lines.push(String(line));
                pendingRows += 1;
                return frame;
            },
            bar(label, fraction, _color) {
                const pct = Math.round(Math.max(0, Math.min(1, fraction)) * 100);
                rec.lines.push(`${label}: ${pct}%`);
                pendingRows += 1;
                return frame;
            },
            tabs(id, names) {
                flushRows();
                const key = String(id);
                const list = asNames(names);
                const selected = String(chrome('tabs', key, list) ?? '');
                rec.tabBands.push({ id: key, names: list, selected });
                return selected;
            },
            cells(cols) {
                rec.lines.push(cols.map((c) => (typeof c === 'string' ? c : c.text)).join(' | '));
                pendingRows += 1;
                return frame;
            },
            strip(id, names, status, brand, resolved) {
                flushRows();
                const key = String(id);
                const list = asNames(names);
                const st = status == null ? '' : String(status);
                const br = brand == null ? '' : String(brand);
                // `resolved` is the jive frame plan's pre-resolved selection;
                // a script's own call resolves it on the chrome store.
                const selected =
                    resolved === undefined
                        ? String(chrome('strip', key, list, st, br) ?? '')
                        : String(resolved);
                rec.strip = { id: key, names: list, status: st, brand: br, selected };
                rec.title = br;
                return selected;
            },
            rail(id, names, resolved) {
                flushRows();
                const key = String(id);
                const list = asNames(names);
                const selected =
                    resolved === undefined
                        ? String(chrome('rail', key, list) ?? '')
                        : String(resolved);
                rec.rail = { id: key, names: list, selected };
                return selected;
            },
            footer(text) {
                rec.footer = String(text);
                return frame;
            },
            rowsLeft() {
                flushRows();
                const n = chrome('rowsLeft');
                return typeof n === 'number' ? n : 0;
            },
            statGrid(rows, columns) {
                flushRows();
                const lines = chrome('statGrid', rows, columns);
                if (lines && typeof lines.length === 'number') {
                    for (let i = 0; i < lines.length; i++) {
                        rec.lines.push(String(lines[i]));
                    }
                }
                return frame;
            },
            select(id, label, options, current) {
                flushRows();
                rec.selects[id] = { label, options, current };
                if (options && options.length > 0) {
                    chrome('select');
                }
                return current;
            },
            buttons(items) {
                flushRows();
                if (!items || items.length === 0) {
                    return null;
                }
                const advertised = [];
                for (const item of items) {
                    if (item == null) continue;
                    advertised.push({
                        id: String(item.id),
                        label: String(item.label),
                    });
                }
                if (advertised.length === 0) {
                    return null;
                }
                for (const btn of advertised) {
                    rec.buttons.push(btn);
                }
                chrome('buttons');
                const click = host().paintClick;
                if (typeof click === 'string' && advertised.some((b) => b.id === click)) {
                    host().paintClick = null;
                    return click;
                }
                return null;
            },
            end() {
                flushRows();
                host().paint = {
                    title: rec.title,
                    accent: rec.accent,
                    lines: rec.lines,
                    buttons: rec.buttons,
                    strip: rec.strip,
                    rail: rec.rail,
                    footer: rec.footer,
                    tabs: rec.tabBands,
                };
            },
        };
        return new Proxy(frame, {
            get(target, prop) {
                if (typeof prop === 'symbol') return target[prop];
                if (prop in target) return target[prop];
                throw notImpl('Paint.' + String(prop));
            },
        });
    },
};
