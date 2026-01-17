use serde::{Deserialize, Serialize};
use vdb_projections::PoolConfig;
use vdb_types::{DataClass, Placement, StreamName};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerityDbConfig {
    pub pool: PoolConfig,
    pub stream_name: StreamName,
    pub data_class: DataClass,
    pub placement: Placement,
}
