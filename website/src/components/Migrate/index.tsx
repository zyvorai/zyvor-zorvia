// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import type {ReactNode} from 'react';
import Heading from '@theme/Heading';
import Link from '@docusaurus/Link';
import Reveal from '@site/src/components/Reveal';

import styles from './styles.module.css';

const CARDS = [
  {
    title: 'Drive it from your AI assistant',
    body: 'Zorvia ships an MCP server so assistants can inspect and operate VMs under the same RBAC.',
    to: '/docs/MCP',
    cta: 'MCP server docs',
  },
];

export default function Migrate(): ReactNode {
  return (
    <section className={styles.section}>
      <div className="container">
        <Reveal>
          <Heading as="h2" className={styles.heading}>
            Works with your tools
          </Heading>
        </Reveal>
        <div className={styles.grid}>
          {CARDS.map((c, i) => (
            <Reveal key={c.title} delay={i * 80}>
              <div className={styles.card}>
                <Heading as="h3">{c.title}</Heading>
                <p>{c.body}</p>
                <Link to={c.to}>{c.cta} →</Link>
              </div>
            </Reveal>
          ))}
        </div>
      </div>
    </section>
  );
}
