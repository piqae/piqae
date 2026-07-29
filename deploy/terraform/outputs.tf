output "server_uri" {
  value = google_cloud_run_v2_service.server.uri
}

output "service_account" {
  value = google_service_account.server.email
}

output "secondary_server_uri" {
  value = try(google_cloud_run_v2_service.server_secondary[0].uri, null)
}

output "global_load_balancer_ip" {
  value = try(google_compute_global_address.global[0].address, null)
}

output "cloud_sql_primary_connection_name" {
  value = try(google_sql_database_instance.primary[0].connection_name, null)
}

output "cloud_sql_dr_connection_name" {
  value = try(google_sql_database_instance.dr_replica[0].connection_name, null)
}

output "managed_object_bucket" {
  value = try(google_storage_bucket.objects[0].name, null)
}
