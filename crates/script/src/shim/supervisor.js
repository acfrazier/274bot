import { Execution } from '../api/execution/Execution.js';

/** Progress the host wedge already tracks via Execution.noteProgress only. */
export const Supervisor = {
    noteProgress() {
        Execution.noteProgress();
    },
};
