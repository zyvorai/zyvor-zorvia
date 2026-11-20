# zorvia_vm module

Reusable Terraform module that creates/destroys a VM through the Zorvia Fabric API.

```hcl
module "web" {
  source        = "../modules/zorvia_vm"
  zorvia_url    = "https://lab:30152"
  zorvia_token  = var.token
  name          = "tf-web-01"
  image         = "quay.io/containerdisks/ubuntu:24.04"
  cpus          = 2
  memory        = 2048
  expose_ssh    = true
}
```

This is the supported GitOps path until a registry-published `zyvorai/zorvia` provider exists.
See `schema.json` from `zorvia dev terraform-scaffold` for the resource contract.
