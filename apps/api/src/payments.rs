//! Payment gateway seam — where finalized invoices leave for an external payments provider.
//!
//! Month-end settlement produces draft invoices in Postgres (`jobs::settle`); the push job
//! (`jobs::push`) hands them here. The provider is abstracted behind [`PaymentGateway`] so the
//! online path and CI never need Stripe credentials or network access: [`LogGateway`] is the
//! credential-free stand-in, and a real Stripe adapter is a deploy-time drop-in — implement the
//! trait and select it in `main` (the same pattern as the KMS `KeyProvider` seam in `secrets`).

/// Invoice statuses used on the push path. The column is free-form `TEXT`; these keep the two
/// spellings that matter in one place.
pub mod invoice_status {
    /// Built by settlement, not yet sent to the payments provider.
    pub const DRAFT: &str = "draft";
    /// Finalized and handed to the provider; awaiting payment.
    pub const OPEN: &str = "open";
}

/// A finalized invoice to hand to the payments provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoicePush {
    pub invoice_id: i64,
    pub tenant_id: i64,
    pub currency: String,
    pub total_cents: i64,
    pub lines: Vec<PushLine>,
    /// Stable per invoice. The provider dedupes retries on this key, so a crash between a successful
    /// remote push and the local status update never creates a second charge.
    pub idempotency_key: String,
}

impl InvoicePush {
    pub fn new(
        invoice_id: i64,
        tenant_id: i64,
        currency: String,
        total_cents: i64,
        lines: Vec<PushLine>,
    ) -> Self {
        InvoicePush {
            invoice_id,
            tenant_id,
            currency,
            total_cents,
            lines,
            idempotency_key: idempotency_key(invoice_id),
        }
    }
}

/// One invoice line as sent to the provider (Stripe invoice item).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushLine {
    pub description: String,
    pub quantity: i64,
    pub unit_cents: i64,
    pub amount_cents: i64,
}

/// The provider's acknowledgement of a pushed invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushReceipt {
    /// External invoice identifier (e.g. a Stripe invoice id), stored on the local invoice row.
    pub external_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PaymentError {
    /// The provider rejected or failed the request. The string is for logs only.
    ///
    /// Constructed by a real gateway adapter (a deploy-time drop-in) and by the failure-path test;
    /// the stand-in `LogGateway` never fails, so the production binary has no constructor yet.
    #[allow(dead_code)]
    #[error("payments: gateway error: {0}")]
    Gateway(String),
}

/// Idempotency key for an invoice push — one per invoice, stable across reruns.
fn idempotency_key(invoice_id: i64) -> String {
    format!("ignition:invoice:{invoice_id}")
}

/// Pushes finalized invoices to a payments provider.
///
/// A real Stripe adapter implements this over the Invoices / Invoice Items API, keyed on
/// [`InvoicePush::idempotency_key`]. Dispatch is static (the job is generic over `G`), so no trait
/// object or extra dependency is needed.
pub trait PaymentGateway {
    fn push_invoice(
        &self,
        push: &InvoicePush,
    ) -> impl std::future::Future<Output = Result<PushReceipt, PaymentError>>;
}

/// Credential-free stand-in: logs the push and returns a deterministic local receipt, no network.
///
/// This is what runs locally and in CI. It exercises the whole push path (selection, payload
/// mapping, status transition, idempotency) end to end without a Stripe account — the real adapter
/// swaps in at deploy time behind the same trait.
pub struct LogGateway;

impl PaymentGateway for LogGateway {
    async fn push_invoice(&self, push: &InvoicePush) -> Result<PushReceipt, PaymentError> {
        tracing::info!(
            invoice_id = push.invoice_id,
            tenant_id = push.tenant_id,
            total_cents = push.total_cents,
            lines = push.lines.len(),
            idempotency_key = %push.idempotency_key,
            "invoice push (log gateway — no external call)"
        );
        Ok(PushReceipt {
            external_id: format!("local:invoice:{}", push.invoice_id),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_lines() -> Vec<PushLine> {
        vec![
            PushLine {
                description: "Platform subscription fee".into(),
                quantity: 1,
                unit_cents: 9900,
                amount_cents: 9900,
            },
            PushLine {
                description: "Performance share · 3 deterministic conversions".into(),
                quantity: 3,
                unit_cents: 200,
                amount_cents: 600,
            },
        ]
    }

    #[test]
    fn idempotency_key_is_stable_per_invoice() {
        let a = InvoicePush::new(42, 1, "USD".into(), 10500, sample_lines());
        let b = InvoicePush::new(42, 1, "USD".into(), 10500, vec![]);
        assert_eq!(a.idempotency_key, "ignition:invoice:42");
        assert_eq!(
            a.idempotency_key, b.idempotency_key,
            "the key depends only on the invoice id, so reruns dedupe"
        );
        assert_ne!(
            a.idempotency_key,
            InvoicePush::new(43, 1, "USD".into(), 0, vec![]).idempotency_key
        );
    }

    #[test]
    fn push_carries_lines_and_total_unchanged() {
        let push = InvoicePush::new(7, 2, "USD".into(), 10500, sample_lines());
        assert_eq!(push.total_cents, 10500);
        assert_eq!(push.lines.len(), 2);
        assert_eq!(push.lines[1].quantity, 3);
        assert_eq!(push.lines[1].amount_cents, 600);
    }

    #[tokio::test]
    async fn log_gateway_returns_a_deterministic_receipt() {
        let push = InvoicePush::new(7, 2, "USD".into(), 10500, sample_lines());
        let receipt = LogGateway.push_invoice(&push).await.unwrap();
        assert_eq!(receipt.external_id, "local:invoice:7");
    }

    /// A gateway that fails — stands in for a real provider outage. The push job rolls its
    /// transaction back on this error, leaving the invoice `draft` for the next run.
    struct FailingGateway;

    impl PaymentGateway for FailingGateway {
        async fn push_invoice(&self, _: &InvoicePush) -> Result<PushReceipt, PaymentError> {
            Err(PaymentError::Gateway("simulated outage".into()))
        }
    }

    #[tokio::test]
    async fn gateway_error_propagates() {
        let push = InvoicePush::new(1, 1, "USD".into(), 0, vec![]);
        let err = FailingGateway.push_invoice(&push).await.unwrap_err();
        assert!(matches!(err, PaymentError::Gateway(_)));
    }
}
