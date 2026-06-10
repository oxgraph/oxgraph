//! Test-only base builders shared by the freeze, backing, and overlay tests.
//!
//! Test fixtures build a base by accumulating a [`WriteOverlay`] over an empty
//! base and freezing the merged view through [`crate::freeze::freeze_view`] —
//! the exact path `create`/`checkpoint` run. These helpers are compiled only
//! under `cfg(test)`.
//!
//! # Performance
//!
//! `perf: unspecified`; test helpers.

use super::{BaseRecords, MergedState, Overlay, WriteOverlay};
use crate::{
    PropertySubject,
    backing::Base,
    catalog::{Catalog, PropertyFamily},
    freeze::{FreezeStamps, freeze_view},
    state::NextIds,
    value::{PropertyType, PropertyValue},
};

/// Freezes a writer delta layered over an empty base into a [`Base`] over
/// owned bytes (the miri-safe attach path), stamping fixed checkpoint
/// values. The writer is the only source of records, so the frozen base
/// holds exactly what the writer created.
///
/// # Performance
///
/// This function is `O(writer change)`.
pub(crate) fn freeze_writer(write: WriteOverlay) -> Base {
    let base = BaseRecords::empty();
    let overlay = write.freeze();
    let view = MergedState::new(&base, &overlay);
    let bytes = freeze_view(
        &view,
        FreezeStamps {
            commit_seq: 1,
            transaction_id: 1,
            generation: 1,
        },
    )
    .expect("freeze writer view");
    Base::open_owned_bytes(bytes).expect("attach base")
}

/// Builds a small canonical base: two labels, one relation type, one role,
/// two element-family property keys, one relation-family key, one
/// incidence-family key, three elements, two relations, two incidences, and
/// several properties — enough to exercise creates, labels, types, every
/// property scalar shape, AND a typed/property-bearing relation plus a
/// property-bearing incidence (so the relation/incidence tombstone index
/// withdrawal paths have base postings to withdraw).
///
/// # Performance
///
/// This function is `O(fixture size)`.
pub(crate) fn small_base() -> Base {
    let mut write = WriteOverlay::new(NextIds::INITIAL, Catalog::empty());
    let base = BaseRecords::empty();
    let person = write.register_label("Person".to_owned()).expect("label");
    let _robot = write.register_label("Robot".to_owned()).expect("label");
    let calls = write
        .register_relation_type("calls".to_owned())
        .expect("rtype");
    let caller = write.register_role("caller".to_owned()).expect("role");
    let name = write
        .register_property_key(
            "name".to_owned(),
            PropertyFamily::Element,
            PropertyType::Text,
        )
        .expect("name key");
    let rank = write
        .register_property_key(
            "rank".to_owned(),
            PropertyFamily::Element,
            PropertyType::Integer,
        )
        .expect("rank key");
    let weight = write
        .register_property_key(
            "weight".to_owned(),
            PropertyFamily::Relation,
            PropertyType::Integer,
        )
        .expect("weight key");
    let slot = write
        .register_property_key(
            "slot".to_owned(),
            PropertyFamily::Incidence,
            PropertyType::Integer,
        )
        .expect("slot key");

    let e1 = write.create_element().expect("e1");
    let e2 = write.create_element().expect("e2");
    let _e3 = write.create_element().expect("e3");
    write.add_element_label(&base, e1, person);
    write.add_element_label(&base, e2, person);

    let r1 = write.create_relation().expect("r1");
    let _r2 = write.create_relation().expect("r2");
    write.set_relation_type(&base, r1, calls);
    let inc1 = write.create_incidence(r1, e1, caller).expect("inc1");
    write.create_incidence(r1, e2, caller).expect("inc2");

    write.set_property(
        &base,
        PropertySubject::Element(e1),
        name,
        PropertyValue::Text("Alice".to_owned()),
    );
    // A relation-family property on the typed relation r1 and an
    // incidence-family property on inc1, so tombstoning each withdraws a real
    // base equality posting (and r1 additionally withdraws its `calls` type).
    write.set_property(
        &base,
        PropertySubject::Relation(r1),
        weight,
        PropertyValue::Integer(3),
    );
    write.set_property(
        &base,
        PropertySubject::Incidence(inc1),
        slot,
        PropertyValue::Integer(1),
    );
    write.set_property(
        &base,
        PropertySubject::Element(e1),
        rank,
        PropertyValue::Integer(-5),
    );
    write.set_property(
        &base,
        PropertySubject::Element(e2),
        name,
        PropertyValue::Text("Bob".to_owned()),
    );

    freeze_writer(write)
}

/// Builds a small `(BaseRecords, Overlay)` pair the freeze test merges: an
/// empty base under an overlay holding two created elements and a property,
/// so the frozen view is non-trivial.
///
/// # Performance
///
/// This function is `O(1)`.
pub(crate) fn base_view_from_ops() -> (BaseRecords, Overlay) {
    let mut write = WriteOverlay::new(NextIds::INITIAL, Catalog::empty());
    let base = BaseRecords::empty();
    let name = write
        .register_property_key(
            "name".to_owned(),
            PropertyFamily::Element,
            PropertyType::Text,
        )
        .expect("name key");
    let e1 = write.create_element().expect("e1");
    let _e2 = write.create_element().expect("e2");
    write.set_property(
        &base,
        PropertySubject::Element(e1),
        name,
        PropertyValue::Text("Alice".to_owned()),
    );
    (base, write.freeze())
}
