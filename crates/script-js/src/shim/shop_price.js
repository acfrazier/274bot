export const SHOP_SELL_MULTIPLIER = 100;
export const SHOP_DELTA = 10;

export function shopBuyPrice(
    baseCost,
    bought,
    sellMultiplier = SHOP_SELL_MULTIPLIER,
    delta = SHOP_DELTA,
) {
    return globalThis.__rs2b0t_shop_price('shopBuyPrice', baseCost, bought, sellMultiplier, delta);
}

export function buyoutCost(
    baseCost,
    stock,
    sellMultiplier = SHOP_SELL_MULTIPLIER,
    delta = SHOP_DELTA,
) {
    return globalThis.__rs2b0t_shop_price('buyoutCost', baseCost, stock, sellMultiplier, delta);
}
