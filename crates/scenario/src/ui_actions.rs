use api::interact::{operation_of, ActionSpec, Interactions, OpTarget, SendResult};
use api::snapshot::{GameSnapshot, MakeProductView, ReadContext};
use client::client::Client;

fn make_x_component(products: &[MakeProductView], product_id: i32) -> Option<i32> {
    products
        .iter()
        .find(|product| product.object_id == product_id)?
        .buttons
        .iter()
        .find(|button| button.quantity == -1)
        .map(|button| button.component_id)
}

fn finish(result: SendResult<'_>, action: &str) -> Result<(), String> {
    match result {
        SendResult::Sent { .. } => Ok(()),
        SendResult::Refused { reason, .. } => Err(format!("{action}: refused {reason:?}")),
    }
}

/// Select Make X for one exact product from the already-open production panel.
/// Chat make panels use the published IF_BUTTON component; main make panels
/// use the existing MakePanel item-operation path. Neither path encodes packets
/// here.
pub(crate) fn make_product(
    snapshot: &GameSnapshot,
    client: &mut Client,
    product_id: i32,
) -> Result<(), String> {
    if !snapshot.make_products().is_empty() {
        let component_id =
            make_x_component(snapshot.make_products(), product_id).ok_or_else(|| {
                format!("Make X product {product_id} is not present in the open make panel")
            })?;
        let read = ReadContext::new(snapshot);
        let widget = read
            .component(component_id)
            .ok_or_else(|| format!("Make X component {component_id} is not published"))?;
        return finish(
            Interactions::new(snapshot, client).if_button(widget),
            "Make X",
        );
    }

    let item = snapshot
        .main_make()
        .iter()
        .find(|item| item.def.id == product_id)
        .ok_or_else(|| {
            format!("Make X product {product_id} is not present in an open make panel")
        })?;
    let target = OpTarget::Item(item);
    let operation = operation_of(&target, "Make X")
        .ok_or_else(|| format!("Make X operation is not posted for product {product_id}"))?;
    finish(
        Interactions::new(snapshot, client).interact(target, ActionSpec::Operation(operation)),
        "MakePanel",
    )
}

/// Answer a currently observed quantity prompt through the normal interaction API.
pub(crate) fn enter_amount(
    snapshot: &GameSnapshot,
    client: &mut Client,
    value: i32,
) -> Result<(), String> {
    finish(
        Interactions::new(snapshot, client).answer_count(value),
        "AnswerCount",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::snapshot::MakeButtonView;

    #[test]
    fn make_x_selection_uses_exact_product_and_x_quantity() {
        let products = [
            MakeProductView {
                object_id: 52,
                name: "Arrow shafts".into(),
                buttons: vec![
                    MakeButtonView {
                        quantity: 1,
                        component_id: 101,
                    },
                    MakeButtonView {
                        quantity: -1,
                        component_id: 104,
                    },
                ],
            },
            MakeProductView {
                object_id: 60,
                name: "Unstrung shortbow".into(),
                buttons: vec![MakeButtonView {
                    quantity: -1,
                    component_id: 204,
                }],
            },
        ];

        assert_eq!(make_x_component(&products, 52), Some(104));
        assert_eq!(make_x_component(&products, 60), Some(204));
        assert_eq!(make_x_component(&products, 53), None);
    }
}
