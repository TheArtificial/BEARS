};
use den_runtime::{
    bears::{db, db::grant_membership, db::BearParams, BearProfile},
};
use den_service::memory_proposals::{create, CreateMemoryProposal};

async fn seed_curate_agent(
    pool: &PgPool,
