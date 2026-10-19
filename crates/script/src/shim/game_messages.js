export const CANT_REACH = /^i can't reach that/i;
export const WRONG_SIDE = /can(?:'t|not) do that from here/i;

// Native owns per-context history and page policy; this shim retains the
// public class, singleton, constants and argument marshaling only.

class GameMessagesImpl {
    record(text) {
        globalThis.__rs2b0t_game_messages(0, text);
    }

    mark() {
        return globalThis.__rs2b0t_game_messages(1);
    }

    since(mark) {
        return globalThis.__rs2b0t_game_messages(2, mark);
    }

    sawSince(mark, pattern) {
        return globalThis.__rs2b0t_game_messages(3, mark, pattern);
    }

    firstSince(mark, pattern) {
        return globalThis.__rs2b0t_game_messages(4, mark, pattern);
    }

    recent(limit = 8) {
        return globalThis.__rs2b0t_game_messages(5, limit);
    }

    reset() {
        globalThis.__rs2b0t_game_messages(6);
    }
}

export const GameMessages = new GameMessagesImpl();
