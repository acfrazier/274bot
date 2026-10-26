import { Trade } from '../../api/trade/Trade.js';

export const SETTINGS = {
    partner: { type: 'string' },
};

export default class TradeBot extends LoopingBot {
    override async loop() {
        if (!Trade.active()) {
            await Trade.request(this.settings.str('partner'));
            return;
        }
        if (Trade.onOfferScreen()) {
            await Trade.offerAll('Coins');
            await Trade.accept();
            return;
        }
        if (Trade.onConfirmScreen()) {
            await Trade.accept();
        }
    }
}
