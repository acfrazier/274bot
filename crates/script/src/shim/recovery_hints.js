export const RecoveryHints = {
    pendingRecovery: false,
    anchor: null,

    takeAnchor() {
        if (!this.pendingRecovery) {
            return null;
        }
        this.pendingRecovery = false;
        return this.anchor;
    },

    clear() {
        this.pendingRecovery = false;
        this.anchor = null;
    },
};
