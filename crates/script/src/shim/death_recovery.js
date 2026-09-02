// No-op DeathRecovery Task: validate() => false so ChickenKiller still Starts.
// The real rs2b0t death-recovery planner is not v1.
export class DeathRecovery {
    constructor(_bot, _opts) {}

    validate() {
        return false;
    }

    async execute() {}
}
