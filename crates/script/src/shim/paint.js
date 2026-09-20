// Recording Paint: begin returns a frame whose methods append to ScriptPaint
// on the host handle. Unused widgets throw `not impl`. Chrome selection,
// rowsLeft budget and statGrid column policy live in the Rust helper.
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
        const frame = {
            title(text) {
                rec.title = String(text);
                chrome('title');
                return frame;
            },
            row(...cols) {
                rec.lines.push(cols.join(' | '));
                chrome('body');
                return frame;
            },
            gap(px) {
                rec.lines.push('');
                chrome('gap', px);
                return frame;
            },
            text(line) {
                rec.lines.push(String(line));
                chrome('body');
                return frame;
            },
            bar(label, fraction, _color) {
                const pct = Math.round(Math.max(0, Math.min(1, fraction)) * 100);
                rec.lines.push(`${label}: ${pct}%`);
                chrome('body');
                return frame;
            },
            tabs(id, names) {
                const key = String(id);
                const list = asNames(names);
                const selected = String(chrome('tabs', key, list) ?? '');
                rec.tabBands.push({ id: key, names: list, selected });
                return selected;
            },
            cells(cols) {
                rec.lines.push(cols.map((c) => (typeof c === 'string' ? c : c.text)).join(' | '));
                chrome('body');
                return frame;
            },
            strip(id, names, status, brand) {
                const key = String(id);
                const list = asNames(names);
                const st = status == null ? '' : String(status);
                const br = brand == null ? '' : String(brand);
                const selected = String(chrome('strip', key, list, st, br) ?? '');
                rec.strip = { id: key, names: list, status: st, brand: br, selected };
                rec.title = br;
                return selected;
            },
            rail(id, names) {
                const key = String(id);
                const list = asNames(names);
                const selected = String(chrome('rail', key, list) ?? '');
                rec.rail = { id: key, names: list, selected };
                return selected;
            },
            footer(text) {
                rec.footer = String(text);
                chrome('footer');
                return frame;
            },
            rowsLeft() {
                const n = chrome('rowsLeft');
                return typeof n === 'number' ? n : 0;
            },
            statGrid(rows, columns) {
                const lines = chrome('statGrid', rows, columns);
                if (lines && typeof lines.length === 'number') {
                    for (let i = 0; i < lines.length; i++) {
                        rec.lines.push(String(lines[i]));
                    }
                }
                return frame;
            },
            select(id, label, options, current) {
                rec.selects[id] = { label, options, current };
                if (options && options.length > 0) {
                    chrome('select');
                }
                return current;
            },
            buttons(items) {
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
