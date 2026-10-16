# Social assets

Rebuildable cards for README, docs Open Graph, and LinkedIn/X. Story matches the README “What you get” five-step strip (Create → Day-2 → Access → Guard → Audit).

| File | Size | Use |
|------|------|-----|
| `zorvia-share-card.png` | 1200×630 | README hero (light), docs site `themeConfig.image`, Twitter/OG |
| `zorvia-share-card-dark.png` | 1200×630 | README hero for dark themes (`<picture>` `prefers-color-scheme: dark`) |
| `zorvia-social-card.jpg` | 1600×900 | LinkedIn / X posts |
| `zorvia-share-card.html` · `zorvia-share-card-dark.html` · `zorvia-social-card.html` | sources | Edit HTML, then rebuild |

## Palette

apple.com blue and white: white → `#f5f5f7` background with a faint blue wash, ink `#1d1d1f`, secondary `#6e6e73`,
hairline `#d2d2d7`, blue `#0071e3` → `#2997ff`. Dark card: `#000` / `#0b0b0f`, text `#f5f5f7`, cards `#1d1d1f`,
blue `#0a84ff` / `#2997ff`. Type is Helvetica Neue with Menlo for labels. Orange `#ff6a2a` appears exactly once per
image, as the small dot on the Create step; the Zyvor mark is drawn inline in blue.

## Rebuild

```bash
./docs/social/build-social-cards.sh
```

Needs Google Chrome and macOS `sips` (override the browser with `CHROME=/path/to/chrome`). The script renders all
outputs above and the comparison cards below.

## GitHub social preview

The repository's **Social preview** image (Settings → Social preview) cannot be set through the API or `gh`;
upload `zorvia-share-card.png` there by hand after it changes.

Licence wording follows `LICENSE` (Apache-2.0). The version chip tracks `Cargo.toml` (currently 0.6.0) and the
CHANGELOG.

## OpenShift vs Zorvia vs ZeusOS cards

Four concepts, each in 1600x900 (LinkedIn/X, JPG), 1200x630 (OG/README, PNG) and 1080x1080 (square, PNG), light and
dark, in `compare/`. Sources: `compare-lanes.html` (hero: "Pick your control plane"), `compare-scorecard.html`,
`compare-poster.html`, `compare-migration.html` (Transiva > h2kvm > GuestKit > Zorvia, Community vs Enterprise), sharing `compare.css` and `compare.js`. Size and theme come from the URL hash, for example
`compare-lanes.html#1200x630-dark`. `./docs/social/build-social-cards.sh` renders all 24.

Every Zorvia row must trace to [FEATURE_MATURITY.md](../FEATURE_MATURITY.md) and OpenShift rows to [leave-openshift.md](../leave-openshift.md).
ZeusOS cells are vendor-stated (https://zyvor.dev/zeus-os; editions and pricing are not public, so say "ask"). Migration-card facts come from the
Transiva and h2kvm `docs/ce-vs-enterprise.md` files. The comment at the top of each card lists its sources. Re-check the "as of" date in the footer and the ZeusOS rows before each new campaign.
ZeusOS is a separate proprietary product, not Zorvia commercial support.

### Post copy

**LinkedIn** (use `compare/lanes-1600x900.jpg`)

> OpenShift Virtualization runs KubeVirt. So does Zorvia. So does ZeusOS.
>
> If you are weighing a Red Hat subscription against your VMs, the workloads do not have to change, only the control plane around them. We wrote the honest comparison, including where OpenShift is the better choice (support contract, certified operators, multi-cluster today).
>
> Zorvia: open source, Apache-2.0, CLI + TUI + web on any KubeVirt cluster. ZeusOS: our separate, proprietary visual workspace. Book a demo or start a 30-day PoC: zyvor.dev/schedule.
>
> github.com/zyvorai/zorvia
>
> #KubeVirt #Kubernetes #OpenShift #Virtualization #OpenSource #PlatformEngineering

**X** (use `compare/poster-1600x900.jpg`)

> Leave the subscription. Keep the VMs.
> OpenShift Virtualization VMs are already KubeVirt objects. Zorvia (Apache-2.0) runs the control plane on any KubeVirt cluster. Honest comparison, OpenShift's wins included: github.com/zyvorai/zorvia #KubeVirt

### Alt text

- **lanes**: Three columns comparing OpenShift (Red Hat subscription, OpenShift only, Red Hat support, Advanced Cluster Management), Zorvia (open source Apache-2.0, any KubeVirt cluster, CLI, TUI and web, multi-cluster Experimental, VMware migration through the separate Transiva and h2kvm suite) and ZeusOS (proprietary commercial, web and TUI, approval-gated changes, Migration Factory, multi-cluster in paid editions). Headline: Same KubeVirt VMs. Pick your control plane.
- **scorecard**: A table with nine rows comparing OpenShift, Zorvia and ZeusOS, marked yes, partial or no. OpenShift leads on support contract, multi-cluster and integrated VMware migration; Zorvia leads on licence cost, running on any KubeVirt cluster, TUI and built-in drift gating, with multi-cluster still Experimental.
- **poster**: Leave the subscription. Keep the VMs. Three cards: OpenShift for Red Hat contract and certified operators, Zorvia open source, ZeusOS proprietary commercial.
- **migration**: Leaving VMware, four tools one path. Transiva exports (Apache-2.0; Enterprise adds CBT, waves and 10+ providers), h2kvm converts and deploys to KubeVirt (production licence; Enterprise adds HA operator, Windows and SAN/Ceph runbooks), GuestKit scores boot readiness (Apache-2.0), Zorvia operates day 2. Zorvia's own importer is Experimental.

**LinkedIn, migration** (use `compare/migration-1600x900.jpg`)

> Leaving VMware for KubeVirt? Export with Transiva, convert with h2kvm, assure with GuestKit, operate on Zorvia. Four tools, each with a free community tier; h2kvm needs a paid licence in production, and Zorvia's own importer is still Experimental. github.com/zyvorai/zorvia #KubeVirt #VMware
