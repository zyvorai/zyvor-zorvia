import {useState} from 'react';
import type {ReactNode} from 'react';
import clsx from 'clsx';
import Heading from '@theme/Heading';
import useBaseUrl from '@docusaurus/useBaseUrl';
import Link from '@docusaurus/Link';
import Reveal from '@site/src/components/Reveal';

import styles from './styles.module.css';

type Step = {
  id: string;
  label: string;
  title: string;
  body: string;
  image: string;
  alt: string;
  doc: string;
};

const STEPS: Step[] = [
  {
    id: 'create',
    label: 'Create',
    title: '43 OS templates, 8 profiles, 5 blueprints',
    body: 'Pick a template and a resource profile, or deploy a whole multi-VM stack from a blueprint. Every create hits live KubeVirt objects.',
    image: '/readme-templates.png',
    alt: 'Zorvia template catalogue',
    doc: '/docs/getting-started',
  },
  {
    id: 'day2',
    label: 'Day-2',
    title: 'Hotplug, live migration, snapshots',
    body: 'Resize disks, hotplug CPU and memory, live migrate, snapshot and restore — with metrics beside every VM.',
    image: '/readme-vm-metrics.png',
    alt: 'Zorvia VM metrics',
    doc: '/docs/SNAPSHOTS',
  },
  {
    id: 'access',
    label: 'Access',
    title: 'Serial console, VNC and in-browser SSH',
    body: 'Reach the guest from the browser or the terminal. Pod logs and exec are one click away.',
    image: '/readme-vm-console.png',
    alt: 'Zorvia VM console',
    doc: '/docs/WEB_CONSOLE',
  },
  {
    id: 'guard',
    label: 'Guard',
    title: 'RBAC, quotas, allow-lists, NetworkPolicy',
    body: 'Namespace allow-lists and quotas keep teams in their lane; admin-only surfaces stay admin-only.',
    image: '/readme-pods-page.png',
    alt: 'Zorvia pods page',
    doc: '/docs/PODS',
  },
  {
    id: 'audit',
    label: 'Audit',
    title: 'A persistent trail you can export',
    body: 'Who did what, when — kept in a persistent audit trail with JSONL export, plus a live event stream.',
    image: '/readme-event-stream.png',
    alt: 'Zorvia event stream',
    doc: '/docs/console-and-api',
  },
];

export default function ProductTour(): ReactNode {
  const [active, setActive] = useState(0);
  const step = STEPS[active];
  const img = useBaseUrl(step.image);
  return (
    <section className={styles.tour} id="tour">
      <div className="container">
        <Reveal>
          <Heading as="h2" className={styles.heading}>
            Create. Operate. Reach. Guard. Prove.
          </Heading>
        </Reveal>
        <div className={styles.tabs} role="tablist" aria-label="Product tour">
          {STEPS.map((s, i) => (
            <button
              key={s.id}
              type="button"
              role="tab"
              id={`tour-tab-${s.id}`}
              aria-selected={i === active}
              aria-controls="tour-panel"
              tabIndex={i === active ? 0 : -1}
              className={clsx(styles.tab, i === active && styles.tabActive)}
              onClick={() => setActive(i)}
              onKeyDown={(e) => {
                if (e.key === 'ArrowRight') setActive((active + 1) % STEPS.length);
                if (e.key === 'ArrowLeft')
                  setActive((active + STEPS.length - 1) % STEPS.length);
              }}>
              {s.label}
            </button>
          ))}
        </div>
        <div
          className={styles.panel}
          role="tabpanel"
          id="tour-panel"
          aria-labelledby={`tour-tab-${step.id}`}>
          <div className={styles.copy}>
            <Heading as="h3">{step.title}</Heading>
            <p>{step.body}</p>
            <Link to={step.doc}>Read the docs →</Link>
          </div>
          <img className={styles.shot} src={img} alt={step.alt} loading="lazy" />
        </div>
      </div>
    </section>
  );
}
