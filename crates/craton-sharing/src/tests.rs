//! Integration tests for the sharing module.

use chrono::Duration;
use serde_json::json;
use craton_types::TenantId;

use crate::{
    AccessToken, ConsentId, ConsentLedger, ConsentRecord, ExportScope, TokenStore,
    TransformationRule, TransformationType, Transformer,
};

#[test]
fn test_end_to_end_export_flow() {
    let tenant = TenantId::new(1);

    // 1. Create consent record
    let mut ledger = ConsentLedger::new();
    let consent = ConsentRecord::new(
        ConsentId::new(0),
        tenant,
        "patient-123",
        "research-org",
        "Research study ABC",
        ExportScope::tables(["encounters"]),
    );
    let consent_id = ledger.record_consent(consent);

    // 2. Create token for the export
    let mut tokens = TokenStore::new();
    let scope = ExportScope::tables(["encounters"])
        .exclude_fields(["ssn"])
        .with_transformation(TransformationRule::new(
            "patient_name",
            TransformationType::Pseudonymize,
        ))
        .with_max_rows(100);

    let token = AccessToken::new(
        tenant,
        "Export for research study ABC",
        scope,
        Duration::hours(24),
    );
    let token_id = tokens.issue(token);

    // 3. Validate token and get scope
    let export_scope = tokens.use_token(token_id).expect("Token should be valid");

    // 4. Verify scope constraints
    assert!(export_scope.allows_table("encounters"));
    assert!(!export_scope.allows_field("ssn"));
    assert!(export_scope.allows_field("patient_name"));

    // 5. Verify consent is still valid
    let consent = ledger
        .get_consent(consent_id)
        .expect("Consent should exist");
    assert!(consent.is_valid());
}

#[test]
fn test_transformation_pipeline() {
    let rules = vec![
        TransformationRule::new("ssn", TransformationType::Redact),
        TransformationRule::new("name", TransformationType::Pseudonymize),
        TransformationRule::new(
            "age",
            TransformationType::Generalize {
                bucket_size: Some(10),
            },
        ),
        TransformationRule::new(
            "phone",
            TransformationType::PartialMask {
                show_start: 0,
                show_end: 4,
            },
        ),
    ];

    let transformer = Transformer::new(rules);

    let mut record = serde_json::Map::new();
    record.insert("ssn".to_string(), json!("123-45-6789"));
    record.insert("name".to_string(), json!("John Doe"));
    record.insert("age".to_string(), json!(42));
    record.insert("phone".to_string(), json!("555-123-4567"));
    record.insert("city".to_string(), json!("Boston"));

    transformer.transform(&mut record).unwrap();

    // SSN should be redacted
    assert_eq!(record.get("ssn"), Some(&serde_json::Value::Null));

    // Name should be pseudonymized
    let name = record.get("name").unwrap().as_str().unwrap();
    assert!(name.starts_with("PSEUDO-"));

    // Age should be generalized
    assert_eq!(record.get("age"), Some(&json!("40-49")));

    // Phone should be partially masked
    let phone = record.get("phone").unwrap().as_str().unwrap();
    assert!(phone.ends_with("4567"));
    assert!(phone.contains("*"));

    // City should be unchanged
    assert_eq!(record.get("city"), Some(&json!("Boston")));
}

#[test]
fn test_token_lifecycle() {
    let mut tokens = TokenStore::new();
    let tenant = TenantId::new(1);

    // Create a limited-use token
    let token = AccessToken::new(
        tenant,
        "Limited token",
        ExportScope::full(),
        Duration::hours(1),
    )
    .with_max_uses(3);

    let token_id = tokens.issue(token);

    // Use it 3 times
    assert!(tokens.use_token(token_id).is_ok());
    assert!(tokens.use_token(token_id).is_ok());
    assert!(tokens.use_token(token_id).is_ok());

    // Fourth use should fail
    assert!(tokens.use_token(token_id).is_err());
}

#[test]
fn test_one_time_token() {
    let mut tokens = TokenStore::new();
    let tenant = TenantId::new(1);

    let token = AccessToken::one_time(
        tenant,
        "One-time export",
        ExportScope::full(),
        Duration::hours(1),
    );

    let token_id = tokens.issue(token);

    // First use should succeed
    assert!(tokens.use_token(token_id).is_ok());

    // Second use should fail
    assert!(tokens.use_token(token_id).is_err());
}

#[test]
fn test_scope_filtering() {
    let scope = ExportScope::tables(["users", "orders"])
        .with_fields(["id", "name", "total"])
        .exclude_fields(["internal_notes"]);

    // Table checks
    assert!(scope.allows_table("users"));
    assert!(scope.allows_table("orders"));
    assert!(!scope.allows_table("secrets"));

    // Field checks
    assert!(scope.allows_field("id"));
    assert!(scope.allows_field("name"));
    assert!(scope.allows_field("total"));
    assert!(!scope.allows_field("internal_notes"));
    assert!(!scope.allows_field("password")); // Not in allowed list

    // Filter function
    let all_fields = vec![
        "id".to_string(),
        "name".to_string(),
        "password".to_string(),
        "internal_notes".to_string(),
    ];
    let filtered = scope.filter_fields(&all_fields);
    assert_eq!(filtered, vec!["id", "name"]);
}

#[test]
fn test_consent_withdrawal() {
    let mut ledger = ConsentLedger::new();
    let tenant = TenantId::new(1);

    let consent = ConsentRecord::new(
        ConsentId::new(0),
        tenant,
        "patient-456",
        "external-lab",
        "Lab result sharing",
        ExportScope::full(),
    );

    let consent_id = ledger.record_consent(consent);

    // Consent should be valid
    assert!(ledger.get_consent(consent_id).unwrap().is_valid());

    // Withdraw consent
    ledger
        .get_consent_mut(consent_id)
        .unwrap()
        .withdraw(Some("Patient requested withdrawal".to_string()));

    // Consent should no longer be valid
    assert!(!ledger.get_consent(consent_id).unwrap().is_valid());

    // Should not find valid consent anymore
    let found = ledger.find_valid_consent(tenant, "patient-456", "external-lab");
    assert!(found.is_none());
}
