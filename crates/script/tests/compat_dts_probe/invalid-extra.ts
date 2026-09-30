import { foodOf, suppliesOf } from '*api/loadout/loadoutPlan.js';

// @ts-expect-error CarryEntry uses item, not name.
foodOf({ food: 'Cake', carry: [{ name: 'Cake', qty: 1 }] }, 'fallback');

// @ts-expect-error CarryEntry uses qty, not count.
foodOf({ food: 'Cake', carry: [{ item: 'Cake', count: 1 }] }, 'fallback');

const supplied = suppliesOf({ name: 'probe', worn: {}, carry: [{ item: 'Cake', qty: 3 }] })[0];
// @ts-expect-error CarryEntry exposes item rather than name.
const wrongName: string = supplied.name;
void wrongName;
