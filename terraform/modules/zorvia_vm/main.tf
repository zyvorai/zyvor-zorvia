terraform {
  required_version = ">= 1.5.0"
}

locals {
  curl_insecure = var.insecure ? "-k" : ""
  payload = jsonencode({
    name       = var.name
    image      = var.image
    cpus       = var.cpus
    memory     = var.memory
    start      = var.start
    expose_ssh = var.expose_ssh
  })
}

resource "terraform_data" "vm" {
  input = local.payload

  provisioner "local-exec" {
    command = <<-EOT
      curl ${local.curl_insecure} -sS -X POST "$ZORVIA_URL/api/vms" \
        -H "Authorization: Bearer $ZORVIA_TOKEN" \
        -H "Content-Type: application/json" \
        -d '$PAYLOAD'
    EOT
    environment = {
      ZORVIA_URL   = var.zorvia_url
      ZORVIA_TOKEN = var.zorvia_token
      PAYLOAD      = local.payload
    }
  }

  provisioner "local-exec" {
    when    = destroy
    command = <<-EOT
      curl ${local.curl_insecure} -sS -X DELETE "$ZORVIA_URL/api/vms/$VM" \
        -H "Authorization: Bearer $ZORVIA_TOKEN"
    EOT
    environment = {
      ZORVIA_URL   = var.zorvia_url
      ZORVIA_TOKEN = var.zorvia_token
      VM           = var.name
    }
  }
}
