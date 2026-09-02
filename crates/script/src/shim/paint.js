// Recording Paint: begin returns a frame whose methods append to ScriptPaint
// on the host handle. Unused widgets throw `not v1`.
import { host, notV1 } from '../shim/_kernel.js';

export const Paint = {
    begin(ctx, opts) {
        const rec = {
            title: null,
            accent: (opts && opts.accent) || null,
            lines: [],
            tabs: {},
            selects: {},
        };
        const frame = {
            title(text) {
                rec.title = String(text);
                return frame;
            },
            row(...cols) {
                rec.lines.push(cols.join(' | '));
                return frame;
            },
            gap() {
                rec.lines.push('');
                return frame;
            },
            text(line) {
                rec.lines.push(String(line));
                return frame;
            },
            bar(label, fraction, _color) {
                const pct = Math.round(Math.max(0, Math.min(1, fraction)) * 100);
                rec.lines.push(`${label}: ${pct}%`);
                return frame;
            },
            tabs(id, names) {
                rec.tabs[id] = names.slice();
                return names[0] ?? '';
            },
            cells(cols) {
                rec.lines.push(cols.map((c) => (typeof c === 'string' ? c : c.text)).join(' | '));
                return frame;
            },
            select(id, label, options, current) {
                rec.selects[id] = { label, options, current };
                return current;
            },
            end() {
                host().paint = {
                    title: rec.title,
                    accent: rec.accent,
                    lines: rec.lines,
                };
            },
        };
        return new Proxy(frame, {
            get(target, prop) {
                if (typeof prop === 'symbol') return target[prop];
                if (prop in target) return target[prop];
                throw notV1('Paint.' + String(prop));
            },
        });
    },
};
