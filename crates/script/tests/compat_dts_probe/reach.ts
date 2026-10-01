import { Reach } from '*api/walking/Reach.js';

// @ts-expect-error ReachEntity requires tile() in addition to actions.
Reach.entityOp({ find: () => ({ interact: () => true, actions: [] }), op: 'Open', expect: () => false, openWhenUnreachable: true });
