import { ChatDialog } from '../ui/dialogue/ChatDialog.js';

export class ContinueDialog {
    constructor(onContinue) {
        this.onContinue = onContinue;
    }

    validate() {
        return ChatDialog.canContinue();
    }

    async execute() {
        this.onContinue?.();
        await ChatDialog.continue();
    }
}
