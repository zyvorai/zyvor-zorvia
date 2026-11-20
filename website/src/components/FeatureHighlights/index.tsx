// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import type {ReactNode} from 'react';
import Link from '@docusaurus/Link';
import Heading from '@theme/Heading';
import styles from './styles.module.css';

type FeatureItem = {
  title: string;
  description: ReactNode;
  to: string;
};

const FeatureList: FeatureItem[] = [
  {
    title: 'CLI, TUI, and web — one API',
    description:
      'Create, snapshot, migrate, backup, and manage access from whichever surface fits the moment, all driving the same backend against real Kubernetes objects.',
    to: '/docs/INTERACTIVE_TUI',
  },
  {
    title: 'Snapshots, done right',
    description:
      'First-class VM snapshot management wired to real KubeVirt/CDI objects, not a parallel bookkeeping system that can drift from the cluster.',
    to: '/docs/SNAPSHOTS',
  },
  {
    title: 'Disk and network management',
    description:
      'Manage VM disks and networking without hand-editing YAML for every routine change.',
    to: '/docs/DISK_MANAGEMENT',
  },
  {
    title: 'Drift guard',
    description:
      "Catch configuration drift between what's declared and what's actually running in the cluster.",
    to: '/docs/DRIFT_GUARD',
  },
  {
    title: 'Golden images & OS templates',
    description:
      'Standardize on golden images and OS templates instead of every VM starting from a bespoke, hand-tuned spec.',
    to: '/docs/GOLDEN_IMAGES',
  },
  {
    title: 'Guest insight & change planning',
    description:
      'See what a change will actually do before you apply it, and what state the guest is actually in — not just what the CRD says.',
    to: '/docs/CHANGE_PLANNER',
  },
];

function Feature({title, description, to}: FeatureItem) {
  return (
    <div className="col col--4">
      <Link to={to} className={styles.card}>
        <Heading as="h3">{title}</Heading>
        <p>{description}</p>
      </Link>
    </div>
  );
}

export default function FeatureHighlights(): ReactNode {
  return (
    <section className={styles.features}>
      <div className="container">
        <div className="row">
          {FeatureList.map((props, idx) => (
            <Feature key={idx} {...props} />
          ))}
        </div>
      </div>
    </section>
  );
}
