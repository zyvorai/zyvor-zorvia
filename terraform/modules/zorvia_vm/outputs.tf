output "name" {
  value = var.name
}

output "console_url" {
  value = "${trimsuffix(var.zorvia_url, "/")}/app/vms/${var.name}/console"
}

output "metrics_url" {
  value = "${trimsuffix(var.zorvia_url, "/")}/api/vms/${var.name}/metrics"
}
