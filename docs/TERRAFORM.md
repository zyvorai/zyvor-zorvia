# Terraform + Zorvia

Zorvia does not yet ship a HashiCorp-published provider binary. The supported
path is a generated scaffold that calls the same Fabric HTTP API as the web
console.

## Generate

```bash
zorvia dev terraform-scaffold --output ./terraform/zorvia-vm \
  --url https://HOST:30152
cd terraform/zorvia-vm
cp terraform.tfvars.example terraform.tfvars
# set zorvia_token from: curl -sk -X POST https://HOST:30152/api/v1/auth/login ...
terraform init
terraform apply
```

## Resources the scaffold drives

| Action | API |
|--------|-----|
| Create VM | `POST /api/vms` |
| Destroy VM | `DELETE /api/vms/:name` |
| Console | `${zorvia_url}/app/vms/${name}/console` |

## Module

`terraform/modules/zorvia_vm` create/destroy a VM through the Fabric API.

```hcl
module "web" {
  source       = "./terraform/modules/zorvia_vm"
  zorvia_url   = var.zorvia_url
  zorvia_token = var.zorvia_token
  name         = "tf-web-01"
}
```

## Provider schema

`terraform-scaffold` writes `schema.json`: the resource contract a future
`zyvorai/zorvia` provider would implement. CRUD maps onto the Fabric HTTP API.

## Why not a full provider yet

A first-class `hashicorp/zorvia` provider needs a published registry module,
acceptance tests against a live cluster, and a stable OpenAPI contract. The
scaffold keeps GitOps users unblocked without inventing a second control plane.
