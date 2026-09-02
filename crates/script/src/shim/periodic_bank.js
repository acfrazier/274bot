// No-op PeriodicBank Task: validate() => false so gold scripts still Start.
export class PeriodicBank {
    constructor(_opts) {}

    validate() {
        return false;
    }

    async execute() {}
}
