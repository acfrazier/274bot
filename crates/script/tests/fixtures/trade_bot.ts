import { Trade } from '../../api/trade/Trade.js';

export const SETTINGS = {
    partner: { type: 'string' },
};

export default class TradeBot extends LoopingBot {
    override loop() {
        if (!Trade.active()) {
            Trade.request(this.settings.str('partner'));
            return;
        }
        if (Trade.onOfferScreen()) {
            Trade.offerAll('Coins');
            Trade.accept();
            return;
        }
        if (Trade.onConfirmScreen()) {
            Trade.accept();
        }
    }
}
