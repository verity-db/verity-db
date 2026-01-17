//! Runtime error types.

/// Errors that can occur during runtime operations.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// Error from the directory (e.g., region not found).
    #[error(transparent)]
    DirectoryError(#[from] vdb_directory::DirectoryError),

    /// Error from VSR consensus.
    #[error(transparent)]
    VsrError(#[from] vdb_vsr::VsrError),

    /// Error from the kernel (e.g., stream already exists).
    #[error(transparent)]
    KernelError(#[from] vdb_kernel::KernelError),

    /// Error from storage (e.g., I/O failure).
    #[error(transparent)]
    StorageError(#[from] vdb_storage::StorageError),

    /// The requested stream was not found.
    #[error("stream not found")]
    StreamNotFound,
}
