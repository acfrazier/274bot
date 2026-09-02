export const CANT_REACH = /^i can't reach that/i;
export const WRONG_SIDE = /can(?:'t|not) do that from here/i;

const CAP = 64;

function matches(pattern, text) {
    pattern.lastIndex = 0;
    return pattern.test(text);
}

class GameMessagesImpl {
    constructor() {
        this.ring = [];
        this.lastSeq = 0;
    }

    record(text) {
        this.ring.push({ seq: ++this.lastSeq, text });
        if (this.ring.length > CAP) {
            this.ring.shift();
        }
    }

    mark() {
        return this.lastSeq;
    }

    since(mark) {
        return this.ring.filter((m) => m.seq > mark);
    }

    sawSince(mark, pattern) {
        return this.ring.some((m) => m.seq > mark && matches(pattern, m.text));
    }
}

export const GameMessages = new GameMessagesImpl();
