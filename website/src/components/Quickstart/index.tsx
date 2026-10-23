import {useState} from 'react';
import type {ReactNode} from 'react';
import Heading from '@theme/Heading';
import Link from '@docusaurus/Link';
import Reveal from '@site/src/components/Reveal';

import styles from './styles.module.css';

const SNIPPET = `zorvia profile show database
zorvia vm create prod-db --template ubuntu-22.04 --cpus 6 --memory 16Gi --disk-size 200Gi
zorvia vm start prod-db
zorvia guest wait-ready prod-db --timeout 120
zorvia vm status prod-db --watch
zorvia blueprint deploy lamp --prefix myapp --start`;

export default function Quickstart(): ReactNode {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(SNIPPET);
      setCopied(true);
      setTimeout(() => setCopied(false), 1800);
    } catch {
      setCopied(false);
    }
  };
  return (
    <section className={styles.section} id="quickstart">
      <div className="container">
        <Reveal>
          <Heading as="h2" className={styles.heading}>
            One real VM in six commands
          </Heading>
          <p className={styles.lead}>
            Profile, create, start, reach the guest. Or deploy a whole
            multi-VM stack from a blueprint.
          </p>
        </Reveal>
        <div className={styles.code}>
          <button type="button" className={styles.copy} onClick={copy} aria-live="polite">
            {copied ? 'Copied ✓' : 'Copy'}
          </button>
          <pre>
            <code>{SNIPPET}</code>
          </pre>
        </div>
        <p className={styles.more}>
          <Link to="/docs/getting-started">Install and full quickstart →</Link>
        </p>
      </div>
    </section>
  );
}
