//! Entity kinds (design section 2.2).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Peer,
    Bind,
    Id,
    IdAcc,
    Refs,
    Anchor,
    Manifest,
    Chunk,
    /// Collaborative-object op (docs/p3/cob-ops.md).
    Cob,
}

const ALL: [Kind; 9] = [
    Kind::Peer,
    Kind::Bind,
    Kind::Id,
    Kind::IdAcc,
    Kind::Refs,
    Kind::Anchor,
    Kind::Manifest,
    Kind::Chunk,
    Kind::Cob,
];

impl Kind {
    pub const fn code(self) -> u8 {
        match self {
            Kind::Peer => 1,
            Kind::Bind => 2,
            Kind::Id => 3,
            Kind::IdAcc => 4,
            Kind::Refs => 5,
            Kind::Anchor => 6,
            Kind::Manifest => 7,
            Kind::Chunk => 8,
            Kind::Cob => 9,
        }
    }

    /// The `ag.kind` attribute value, also the tag of the content hash.
    pub const fn name(self) -> &'static str {
        match self {
            Kind::Peer => "peer",
            Kind::Bind => "bind",
            Kind::Id => "id",
            Kind::IdAcc => "idacc",
            Kind::Refs => "refs",
            Kind::Anchor => "anchor",
            Kind::Manifest => "manifest",
            Kind::Chunk => "chunk",
            Kind::Cob => "cob",
        }
    }

    /// Per-object composite signature context, `arkiv-git/<kind>/v1`
    /// (`docs/p0/pq-signatures.md` section 1 usage rules).
    pub const fn sig_ctx(self) -> &'static [u8] {
        match self {
            Kind::Peer => b"arkiv-git/peer/v1",
            Kind::Bind => b"arkiv-git/bind/v1",
            Kind::Id => b"arkiv-git/id/v1",
            Kind::IdAcc => b"arkiv-git/idacc/v1",
            Kind::Refs => b"arkiv-git/refs/v1",
            Kind::Anchor => b"arkiv-git/anchor/v1",
            Kind::Manifest => b"arkiv-git/manifest/v1",
            Kind::Chunk => b"arkiv-git/chunk/v1",
            Kind::Cob => b"arkiv-git/cob/v1",
        }
    }

    pub fn from_code(code: u64) -> Option<Kind> {
        ALL.into_iter().find(|k| u64::from(k.code()) == code)
    }

    pub fn from_name(name: &str) -> Option<Kind> {
        ALL.into_iter().find(|k| k.name() == name)
    }

    /// Created by the control address (and kept alive forever) vs the bulk address.
    pub const fn plane(self) -> Plane {
        match self {
            Kind::Refs | Kind::Manifest | Kind::Chunk => Plane::Bulk,
            _ => Plane::Control,
        }
    }
}

/// Which per-repo EVM address creates a kind (design section 3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plane {
    Control,
    Bulk,
}
