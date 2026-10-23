#![deny(clippy::disallowed_types)]
#![deny(clippy::disallowed_methods)]
#![deny(clippy::disallowed_macros)]

// probe: METHOD
pub fn resolved_method() {
    let _ = api::prot::Send::close_modal();
}

// probe: FREE_FN
pub fn resolved_free_function() {
    let _ = api::prot::legal_sends_for(client::io::ClientRevision::R289);
}

// probe: EXTERNAL_METHOD_ALIAS
pub fn external_method_alias() {
    let _ = nav_clippy_external_facade::SendAlias::close_modal();
}

// probe: EXTERNAL_METHOD_PROJECTION
pub fn external_method_projection() {
    let _ = <<nav_clippy_external_facade::Projection as nav_clippy_external_facade::Project>::Item>::close_modal();
}

// probe: G8
pub fn associated_type_projection_method() {
    use nav_clippy_external_facade::review::g8_associated_type_projection as g8;
    let _ = <<g8::X as g8::T>::A>::close_modal();
}

// probe: EXTERNAL_INFERRED_VALUE_METHOD
pub fn external_inferred_value_method() {
    let _ = nav_clippy_external_facade::values::static_value::S.named_packet();
}

// probe: EXTERNAL_MACRO_TYPE
pub mod external_macro_type {
    nav_clippy_external_facade::export_client_type!();
}

// probe: B1
pub fn external_static_value_read() {
    let _ = &nav_clippy_external_facade::values::static_value::S;
}

// probe: B2
pub fn external_const_value_read() {
    let _ = nav_clippy_external_facade::values::const_value::C;
}

// probe: B3
pub fn external_associated_const_read() {
    let _ = nav_clippy_external_facade::values::associated_value::S::C;
}

// probe: D6
pub use r#client::io::client_prot::ClientProt as RawPlannerClientProt;

// probe: E5
#[cfg(test)]
pub extern crate client as c;
