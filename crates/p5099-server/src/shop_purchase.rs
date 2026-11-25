use p5099_core::shop_protocol::{
    SHOP_RESULT_ALREADY_OWNED, SHOP_RESULT_NOT_ENOUGH_CASH, SHOP_RESULT_NOT_ENOUGH_LUCCI,
    SHOP_RESULT_NOT_ON_SALE, SHOP_RESULT_NOT_READY,
};
use p5099_profile::{Profile, is_stackable_item};

use crate::shop_catalog::{ShopCurrency, ShopStock};

pub(crate) const MAX_ITEM_AMOUNT: u16 = i16::MAX as u16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShopPurchasePlan {
    pub currency: ShopCurrency,
    pub price: u32,
    pub amounts: Vec<(u16, u16, u16)>,
}

pub(crate) fn current_amount(profile: &Profile, category: u16, id: u16) -> u16 {
    profile
        .item_amount(category, id)
        .unwrap_or(profile.rider.slot_changer)
}

pub(crate) fn plan_shop_purchase(
    stock: &ShopStock,
    granted: impl Fn(u16, u16) -> bool,
    profile: &Profile,
) -> Result<ShopPurchasePlan, u32> {
    if !stock.on_sale {
        return Err(SHOP_RESULT_NOT_ON_SALE);
    }
    if matches!(stock.currency, ShopCurrency::Other(_)) || stock.items.is_empty() {
        return Err(SHOP_RESULT_NOT_READY);
    }
    let mut amounts: Vec<(u16, u16, u16)> = Vec::new();
    for item in &stock.items {
        if !granted(item.category, item.id) {
            return Err(SHOP_RESULT_NOT_READY);
        }
        if !is_stackable_item(item.category, item.id) {
            continue;
        }
        let current = amounts
            .iter()
            .find(|(category, id, _)| *category == item.category && *id == item.id)
            .map_or_else(
                || current_amount(profile, item.category, item.id),
                |(_, _, amount)| *amount,
            );
        let next = current.saturating_add(item.count).min(MAX_ITEM_AMOUNT);
        if next == current {
            continue;
        }
        match amounts
            .iter_mut()
            .find(|(category, id, _)| *category == item.category && *id == item.id)
        {
            Some(entry) => entry.2 = next,
            None => amounts.push((item.category, item.id, next)),
        }
    }
    if amounts.is_empty() {
        return Err(SHOP_RESULT_ALREADY_OWNED);
    }
    let balance = match stock.currency {
        ShopCurrency::Cash => profile.rider.cash,
        ShopCurrency::Lucci => profile.rider.lucci,
        ShopCurrency::Other(_) => unreachable!("rejected above"),
    };
    if balance < stock.price {
        return Err(match stock.currency {
            ShopCurrency::Lucci => SHOP_RESULT_NOT_ENOUGH_LUCCI,
            _ => SHOP_RESULT_NOT_ENOUGH_CASH,
        });
    }
    Ok(ShopPurchasePlan {
        currency: stock.currency,
        price: stock.price,
        amounts,
    })
}

pub(crate) fn apply_shop_purchase(profile: &mut Profile, plan: &ShopPurchasePlan) {
    match plan.currency {
        ShopCurrency::Cash => profile.rider.cash -= plan.price,
        ShopCurrency::Lucci => profile.rider.lucci -= plan.price,
        ShopCurrency::Other(_) => {}
    }
    for &(category, id, amount) in &plan.amounts {
        profile.set_item_amount(category, id, amount);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shop_catalog::ShopStockItem;

    fn catalog() -> impl Fn(u16, u16) -> bool {
        |category, id| matches!((category, id), (3, 1426) | (23, 108))
    }

    fn stock(currency: ShopCurrency, price: u32, items: &[(u16, u16, u16)]) -> ShopStock {
        ShopStock {
            price,
            currency,
            on_sale: true,
            items: items
                .iter()
                .map(|&(category, id, count)| ShopStockItem {
                    category,
                    id,
                    count,
                })
                .collect(),
        }
    }

    fn profile() -> Profile {
        let mut profile = Profile::default();
        profile.rider.slot_changer = 100;
        profile.rider.cash = 1_000;
        profile.rider.lucci = 500;
        profile
    }

    #[test]
    fn owned_unit_items_are_refused_without_a_charge() {
        let surf = stock(ShopCurrency::Cash, 1_100, &[(3, 1426, 1)]);
        assert_eq!(
            plan_shop_purchase(&surf, catalog(), &profile()),
            Err(SHOP_RESULT_ALREADY_OWNED)
        );
    }

    #[test]
    fn consumables_add_to_the_tracked_count_and_charge_the_currency() {
        let hourglass = stock(ShopCurrency::Lucci, 200, &[(23, 108, 10)]);
        let mut profile = profile();
        let plan = plan_shop_purchase(&hourglass, catalog(), &profile).unwrap();
        assert_eq!(plan.amounts, [(23, 108, 110)]);
        apply_shop_purchase(&mut profile, &plan);
        assert_eq!(
            (profile.rider.lucci, profile.item_amount(23, 108)),
            (300, Some(110))
        );

        let plan = plan_shop_purchase(&hourglass, catalog(), &profile).unwrap();
        assert_eq!(plan.amounts, [(23, 108, 120)]);
    }

    #[test]
    fn full_stacks_balance_and_availability_are_checked() {
        let mut full = profile();
        full.set_item_amount(23, 108, MAX_ITEM_AMOUNT);
        let hourglass = stock(ShopCurrency::Cash, 200, &[(23, 108, 10)]);
        assert_eq!(
            plan_shop_purchase(&hourglass, catalog(), &full),
            Err(SHOP_RESULT_ALREADY_OWNED)
        );

        let pricey = stock(ShopCurrency::Cash, 5_000, &[(23, 108, 10)]);
        assert_eq!(
            plan_shop_purchase(&pricey, catalog(), &profile()),
            Err(SHOP_RESULT_NOT_ENOUGH_CASH)
        );
        let pricey_lucci = stock(ShopCurrency::Lucci, 5_000, &[(23, 108, 10)]);
        assert_eq!(
            plan_shop_purchase(&pricey_lucci, catalog(), &profile()),
            Err(SHOP_RESULT_NOT_ENOUGH_LUCCI)
        );

        let unsafe_item = stock(ShopCurrency::Cash, 1, &[(23, 999, 1)]);
        assert_eq!(
            plan_shop_purchase(&unsafe_item, catalog(), &profile()),
            Err(SHOP_RESULT_NOT_READY)
        );
        let other = stock(ShopCurrency::Other(3), 1, &[(23, 108, 1)]);
        assert_eq!(
            plan_shop_purchase(&other, catalog(), &profile()),
            Err(SHOP_RESULT_NOT_READY)
        );
        let mut off_sale = hourglass;
        off_sale.on_sale = false;
        assert_eq!(
            plan_shop_purchase(&off_sale, catalog(), &profile()),
            Err(SHOP_RESULT_NOT_ON_SALE)
        );
    }
}
