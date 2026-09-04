// Hook runner. No gathering eat planner.
export const Sustain = {
    hook: null,
    running: false,
    set(hook) {
        this.hook = hook;
    },
    async run() {
        if (!this.hook || this.running) return;
        this.running = true;
        try {
            await this.hook();
        } finally {
            this.running = false;
        }
    },
};
