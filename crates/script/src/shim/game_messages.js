import { snap, notImpl } from '../../shim/_kernel.js';

export const CANT_REACH = /^i can't reach that/i;
export const WRONG_SIDE = /can(?:'t|not) do that from here/i;

function matches(pattern, text) {
    pattern.lastIndex = 0;
    return pattern.test(text);
}

function lines() {
    return snap().chat_lines || [];
}

class GameMessagesImpl {
    record(text) {
        throw notImpl('GameMessages.record');
    }

    mark() {
        const rs = lines();
        if (rs.length === 0) return 0;
        return rs.reduce((m, l) => Math.max(m, typeof l.seq === 'number' ? l.seq : 0), 0);
    }

    since(mark) {
        return lines()
            .filter((m) => typeof m.seq === 'number' && m.seq > mark)
            .map((m) => ({ seq: m.seq, text: m.text }));
    }

    sawSince(mark, pattern) {
        return lines().some((m) => typeof m.seq === 'number' && m.seq > mark && matches(pattern, m.text));
    }

    firstSince(mark, pattern) {
        throw notImpl('GameMessages.firstSince');
    }

    recent(limit = 8) {
        throw notImpl('GameMessages.recent');
    }

    reset() {
        throw notImpl('GameMessages.reset');
    }
}

export const GameMessages = new GameMessagesImpl();
