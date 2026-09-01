// Recording Paint: `begin` returns a frame whose title/row/gap/end append
// to a ScriptPaint on the host handle. No canvas; `list`/`click` and other
// widgets throw `not v1`.
const host = () => globalThis.__rs2b0t_host || {};
const notV1 = (name) => new Error('not v1: ' + name);

export const Paint = {
    begin(ctx, opts) {
        const rec = {
            title: null,
            accent: (opts && opts.accent) || null,
            lines: [],
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
