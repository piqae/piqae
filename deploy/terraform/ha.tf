locals {
  secondary_enabled = var.enable_multi_region
  runtime_env = {
    PIQAE_ENVIRONMENT               = var.environment
    PIQAE_DEPLOYMENT                = "cloud"
    PIQAE_RUN_MIGRATIONS_ON_STARTUP = "false"
    PIQAE_IDENTITY_PROVIDER         = "workos"
    PIQAE_BILLING_ENABLED           = "true"
    STRIPE_METER_EVENT_NAME         = var.stripe_meter_event_name
    PIQAE_BIND                      = "0.0.0.0:8080"
    PIQAE_AUTH_MODE                 = var.auth_mode
    PIQAE_OIDC_ISSUER               = var.oidc_issuer
    PIQAE_OIDC_JWKS_URL             = var.oidc_jwks_url
    PIQAE_OIDC_AUDIENCE             = var.oidc_audience
    PIQAE_OIDC_BINDING_CLAIM        = var.oidc_binding_claim
    PIQAE_OIDC_BINDING_VALUE        = var.oidc_binding_value
    PIQAE_OIDC_ORGANIZATION_CLAIM   = var.oidc_organization_claim
    PIQAE_OIDC_PERMISSIONS_CLAIM    = var.oidc_permissions_claim
    PIQAE_OIDC_ALLOW_UNRESTRICTED   = "false"
    PIQAE_OBJECT_STORE              = var.enable_managed_data_plane ? "gcs" : "s3"
    PIQAE_GCS_BUCKET                = var.enable_managed_data_plane ? google_storage_bucket.objects[0].name : ""
    PIQAE_S3_ENDPOINT               = var.object_store_endpoint
    PIQAE_S3_BUCKET                 = var.object_store_bucket
    PIQAE_S3_REGION                 = "auto"
  }
}

resource "google_project_service" "compute" {
  count              = var.enable_global_load_balancer ? 1 : 0
  service            = "compute.googleapis.com"
  disable_on_destroy = false
}

resource "google_project_service" "sqladmin" {
  count              = var.enable_managed_data_plane ? 1 : 0
  service            = "sqladmin.googleapis.com"
  disable_on_destroy = false
}

resource "google_project_service" "storage" {
  count              = var.enable_managed_data_plane ? 1 : 0
  service            = "storage.googleapis.com"
  disable_on_destroy = false
}

resource "google_cloud_run_v2_service" "server_secondary" {
  for_each            = local.secondary_enabled ? toset(["api", "sync", "worker"]) : toset([])
  name                = "${local.name}-${var.secondary_region}-${each.key}"
  location            = var.secondary_region
  deletion_protection = var.environment == "production"
  ingress             = var.enable_global_load_balancer ? "INGRESS_TRAFFIC_INTERNAL_LOAD_BALANCER" : "INGRESS_TRAFFIC_ALL"

  depends_on = [google_project_service.run]

  template {
    service_account                  = google_service_account.server.email
    timeout                          = "60s"
    max_instance_request_concurrency = 4

    dynamic "volumes" {
      for_each = var.enable_managed_data_plane ? [1] : []
      content {
        name = "cloudsql"
        cloud_sql_instance {
          instances = [google_sql_database_instance.primary[0].connection_name]
        }
      }
    }

    scaling {
      min_instance_count = var.environment == "production" ? var.secondary_min_instances : 0
      max_instance_count = local.max_instances
    }

    containers {
      image = var.image

      ports {
        name           = "h2c"
        container_port = 8080
      }

      resources {
        limits   = { cpu = "1", memory = "1Gi" }
        cpu_idle = false
      }

      dynamic "volume_mounts" {
        for_each = var.enable_managed_data_plane ? [1] : []
        content {
          name       = "cloudsql"
          mount_path = "/cloudsql"
        }
      }

      dynamic "env" {
        for_each = local.runtime_env
        content {
          name  = env.key
          value = env.value
        }
      }
      env {
        name  = "PIQAE_SERVICE_ROLE"
        value = each.key
      }

      dynamic "env" {
        for_each = {
          PIQAE_DATABASE_URL         = google_secret_manager_secret.database_url.secret_id
          PIQAE_S3_ACCESS_KEY_ID     = google_secret_manager_secret.object_access_key.secret_id
          PIQAE_S3_SECRET_ACCESS_KEY = google_secret_manager_secret.object_secret_key.secret_id
          PIQAE_WEBHOOK_MASTER_KEY   = google_secret_manager_secret.webhook_master_key.secret_id
          STRIPE_SECRET_KEY          = google_secret_manager_secret.stripe_secret_key.secret_id
          STRIPE_WEBHOOK_SECRET      = google_secret_manager_secret.stripe_webhook_secret.secret_id
        }
        content {
          name = env.key
          value_source {
            secret_key_ref {
              secret  = env.value
              version = "latest"
            }
          }
        }
      }

      startup_probe {
        failure_threshold = 12
        period_seconds    = 2
        timeout_seconds   = 1
        http_get {
          path = "/v1/ready"
          port = 8080
        }
      }
      liveness_probe {
        failure_threshold = 3
        period_seconds    = 10
        timeout_seconds   = 2
        http_get {
          path = "/v1/health"
          port = 8080
        }
      }
    }
  }

  lifecycle {
    precondition {
      condition     = !var.enable_global_load_balancer || var.enable_multi_region
      error_message = "The global load balancer requires enable_multi_region=true."
    }
  }

  traffic {
    type    = "TRAFFIC_TARGET_ALLOCATION_TYPE_LATEST"
    percent = 100
  }
}

resource "google_cloud_run_v2_service_iam_member" "secondary_invoker" {
  for_each = local.secondary_enabled && var.allow_public_cloud_run_invocation ? toset(["api", "sync"]) : toset([])
  project  = var.gcp_project_id
  location = google_cloud_run_v2_service.server_secondary[each.key].location
  name     = google_cloud_run_v2_service.server_secondary[each.key].name
  role     = "roles/run.invoker"
  member   = "allUsers"
}

resource "google_compute_region_network_endpoint_group" "primary" {
  count                 = var.enable_global_load_balancer ? 1 : 0
  name                  = "${local.name}-primary"
  region                = var.gcp_region
  network_endpoint_type = "SERVERLESS"
  cloud_run { service = google_cloud_run_v2_service.server["api"].name }
  depends_on = [google_project_service.compute]
}

resource "google_compute_region_network_endpoint_group" "secondary" {
  count                 = var.enable_global_load_balancer ? 1 : 0
  name                  = "${local.name}-secondary"
  region                = var.secondary_region
  network_endpoint_type = "SERVERLESS"
  cloud_run { service = google_cloud_run_v2_service.server_secondary["api"].name }
  depends_on = [google_project_service.compute]
}

resource "google_compute_region_network_endpoint_group" "primary_sync" {
  count                 = var.enable_global_load_balancer ? 1 : 0
  name                  = "${local.name}-primary-sync"
  region                = var.gcp_region
  network_endpoint_type = "SERVERLESS"
  cloud_run { service = google_cloud_run_v2_service.server["sync"].name }
  depends_on = [google_project_service.compute]
}

resource "google_compute_region_network_endpoint_group" "secondary_sync" {
  count                 = var.enable_global_load_balancer ? 1 : 0
  name                  = "${local.name}-secondary-sync"
  region                = var.secondary_region
  network_endpoint_type = "SERVERLESS"
  cloud_run { service = google_cloud_run_v2_service.server_secondary["sync"].name }
  depends_on = [google_project_service.compute]
}

resource "google_compute_backend_service" "global" {
  count                 = var.enable_global_load_balancer ? 1 : 0
  name                  = "${local.name}-global"
  protocol              = "HTTP"
  load_balancing_scheme = "EXTERNAL_MANAGED"
  backend { group = google_compute_region_network_endpoint_group.primary[0].id }
  backend { group = google_compute_region_network_endpoint_group.secondary[0].id }
  outlier_detection {
    consecutive_errors = 5
    interval {
      seconds = 5
    }
    base_ejection_time {
      seconds = 30
    }
    max_ejection_percent         = 50
    enforcing_consecutive_errors = 100
  }
}

resource "google_compute_backend_service" "global_sync" {
  count                 = var.enable_global_load_balancer ? 1 : 0
  name                  = "${local.name}-global-sync"
  protocol              = "HTTP"
  load_balancing_scheme = "EXTERNAL_MANAGED"
  backend { group = google_compute_region_network_endpoint_group.primary_sync[0].id }
  backend { group = google_compute_region_network_endpoint_group.secondary_sync[0].id }
  outlier_detection {
    consecutive_errors = 5
    interval { seconds = 5 }
    base_ejection_time { seconds = 30 }
    max_ejection_percent         = 50
    enforcing_consecutive_errors = 100
  }
}

resource "google_compute_url_map" "global" {
  count           = var.enable_global_load_balancer ? 1 : 0
  name            = local.name
  default_service = google_compute_backend_service.global[0].id

  host_rule {
    hosts        = ["*"]
    path_matcher = "piqae-routes"
  }

  path_matcher {
    name            = "piqae-routes"
    default_service = google_compute_backend_service.global[0].id
    path_rule {
      paths = [
        "/v1/agent/sync",
        "/v1/agent/jobs/*",
        "/v1/agents/enrol",
      ]
      service = google_compute_backend_service.global_sync[0].id
    }
  }
}

resource "google_compute_managed_ssl_certificate" "global" {
  count = var.enable_global_load_balancer ? 1 : 0
  name  = local.name
  managed { domains = var.load_balancer_domains }
}

resource "google_compute_target_https_proxy" "global" {
  count            = var.enable_global_load_balancer ? 1 : 0
  name             = local.name
  url_map          = google_compute_url_map.global[0].id
  ssl_certificates = [google_compute_managed_ssl_certificate.global[0].id]
}

resource "google_compute_global_address" "global" {
  count = var.enable_global_load_balancer ? 1 : 0
  name  = local.name
}

resource "google_compute_global_forwarding_rule" "https" {
  count                 = var.enable_global_load_balancer ? 1 : 0
  name                  = "${local.name}-https"
  ip_address            = google_compute_global_address.global[0].id
  port_range            = "443"
  ip_protocol           = "TCP"
  network_tier          = "PREMIUM"
  load_balancing_scheme = "EXTERNAL_MANAGED"
  target                = google_compute_target_https_proxy.global[0].id
}

resource "google_sql_database_instance" "primary" {
  count               = var.enable_managed_data_plane ? 1 : 0
  name                = "${local.name}-postgres"
  region              = var.gcp_region
  database_version    = "POSTGRES_16"
  deletion_protection = var.environment == "production"
  depends_on          = [google_project_service.sqladmin]

  settings {
    edition           = "ENTERPRISE_PLUS"
    tier              = var.cloud_sql_tier
    availability_type = "REGIONAL"
    disk_type         = "PD_SSD"
    disk_autoresize   = true
    disk_size         = var.cloud_sql_disk_size_gb
    backup_configuration {
      enabled                        = true
      point_in_time_recovery_enabled = true
      start_time                     = "12:00"
      transaction_log_retention_days = 7
      backup_retention_settings {
        retained_backups = 14
        retention_unit   = "COUNT"
      }
    }
    maintenance_window {
      day          = 7
      hour         = 14
      update_track = "stable"
    }
    insights_config {
      query_insights_enabled  = true
      record_application_tags = true
      record_client_address   = false
    }
    ip_configuration { ipv4_enabled = true }
  }
}

resource "random_password" "database" {
  count   = var.enable_managed_data_plane ? 1 : 0
  length  = 32
  special = false
}

resource "google_sql_database" "piqae" {
  count    = var.enable_managed_data_plane ? 1 : 0
  name     = "piqae"
  instance = google_sql_database_instance.primary[0].name
}

resource "google_sql_user" "piqae" {
  count    = var.enable_managed_data_plane ? 1 : 0
  name     = "piqae"
  instance = google_sql_database_instance.primary[0].name
  password = random_password.database[0].result
}

resource "google_sql_database_instance" "dr_replica" {
  count                = var.enable_managed_data_plane && var.enable_multi_region ? 1 : 0
  name                 = "${local.name}-postgres-dr"
  region               = var.secondary_region
  database_version     = "POSTGRES_16"
  master_instance_name = google_sql_database_instance.primary[0].name
  deletion_protection  = var.environment == "production"

  replica_configuration { failover_target = false }
  settings {
    edition         = "ENTERPRISE_PLUS"
    tier            = var.cloud_sql_tier
    disk_type       = "PD_SSD"
    disk_autoresize = true
    ip_configuration { ipv4_enabled = true }
  }
}

resource "google_storage_bucket" "objects" {
  count                       = var.enable_managed_data_plane ? 1 : 0
  name                        = var.managed_object_bucket_name
  location                    = "AU"
  storage_class               = "STANDARD"
  uniform_bucket_level_access = true
  public_access_prevention    = "enforced"
  force_destroy               = false
  depends_on                  = [google_project_service.storage]

  custom_placement_config {
    data_locations = [var.gcp_region, var.secondary_region]
  }
  versioning { enabled = true }
  lifecycle_rule {
    condition {
      age                = 30
      num_newer_versions = 3
      with_state         = "ARCHIVED"
    }
    action {
      type = "Delete"
    }
  }
}

resource "google_storage_bucket_iam_member" "runtime_objects" {
  count  = var.enable_managed_data_plane ? 1 : 0
  bucket = google_storage_bucket.objects[0].name
  role   = "roles/storage.objectAdmin"
  member = "serviceAccount:${google_service_account.server.email}"
}
