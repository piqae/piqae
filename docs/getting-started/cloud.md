# Cloud evaluation

**Status:** infrastructure foundation implemented; no public Spool SaaS is
claimed as Supported.

The repository contains a GCP Cloud Run Terraform foundation in
[`deploy/terraform`](../../deploy/terraform/README.md). It defaults to Sydney,
external PostgreSQL, external S3-compatible storage, and WorkOS-compatible
OIDC. Optional resources model Melbourne compute, a global HTTPS load balancer,
Cloud SQL HA/DR, and dual-region object storage.

## Safe evaluation path

1. Use a dedicated non-production GCP project and Terraform state.
2. Publish immutable server and dashboard images from a reviewed commit.
3. Supply PostgreSQL, object-store, webhook, and OIDC secrets through an
   encrypted apply workflow.
4. Run `terraform init`, `terraform validate`, and review a saved plan.
5. Apply only after checking deletion protection, quotas, DNS, certificates,
   Cloud Run ingress, and expected monthly cost.
6. Enrol a disposable node using the documented [pairing flow](../nodes/pairing.md).
7. Submit a test PDF and verify its [job state sequence](../printing/jobs-and-statuses.md).

The Terraform module does not create WorkOS applications, DNS records,
production database users, or automatic database promotion. A regional load
balancer cannot make an unavailable PostgreSQL primary writable; follow the
[high-availability runbook](../operations/high-availability.md).

For the intended developer-first product shape, see
[`12-open-source-saas-and-build-plan.md`](../12-open-source-saas-and-build-plan.md).
