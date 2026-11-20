# Namespace restriction for users

Admins set a per-user namespace allow-list; everything else is unchanged.

```
PUT /api/v1/users/{id}/namespaces   {"namespaces": ["team-a", "team-b"]}   # restrict
PUT /api/v1/users/{id}/namespaces   {"namespaces": null}                   # unrestrict
GET /api/v1/users/{id}/namespaces
```

- No list (the default, and every existing user) = unrestricted, as before.
- Admin accounts cannot be restricted. PAM sessions are not restricted.
- **API tokens** can be restricted too: `POST /api/v1/api-tokens` takes an optional `namespaces`
  list (same names and limits as users; omit for unrestricted), shown in the token list. A
  restricted token is held to the same VM-centric route allow-list and namespace checks as a
  restricted user, whatever its role or scopes. An empty list confines it to nothing. A list
  cannot be changed after creation: revoke and re-issue.
- A restricted user may only use VM-centric routes (`/vms*`, `/v1/vms*`, `/v1/snapshots*`,
  `/snapshots*`, `/images*`, `/templates*`, `/v1/auth/*`, health/features/instance).
  Every other route (audit, events, storage, Atlas, dashboards, ...) is 403: those return
  cluster-wide data and fail closed.
- `/v1/vms/{ns}/...`, `/v1/snapshots/{ns}/...` and `?namespace=` must name an allowed
  namespace; `all_namespaces=true` is refused.
- Routes without a namespace (`/vms/{name}/...`, console/VNC/SSH sockets) act on the
  server's default namespace, so a restricted user needs it in their list to use them.
  Users confined to other namespaces should use the `/v1/vms/{ns}/...` routes.
- The allow-list is read on every request, so changes apply immediately.
