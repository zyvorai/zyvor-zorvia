import type {ReactNode} from 'react';
import Heading from '@theme/Heading';
import Link from '@docusaurus/Link';
import Reveal from '@site/src/components/Reveal';

import styles from './styles.module.css';

const CARDS = [
  {
    title: 'Leaving OpenShift Virtualization',
    body: 'An honest comparison: what maps cleanly, what does not, and how to move.',
    to: '/docs/leave-openshift',
    cta: 'Read the comparison',
  },
  {
    title: 'Leaving Nutanix',
    body: 'Export from Transiva, convert with h2kvm, land on KubeVirt — with an honest status.',
    to: '/docs/leave-nutanix',
    cta: 'See the path',
  },
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
            Coming from somewhere else?
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
