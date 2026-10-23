import type {ReactNode} from 'react';
import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useBaseUrl from '@docusaurus/useBaseUrl';
import Layout from '@theme/Layout';
import Heading from '@theme/Heading';
import FeatureHighlights from '@site/src/components/FeatureHighlights';
import ProductTour from '@site/src/components/ProductTour';
import Quickstart from '@site/src/components/Quickstart';
import Migrate from '@site/src/components/Migrate';

import styles from './index.module.css';

function HomepageHeader() {
  const dashboard = useBaseUrl('/readme-dashboard.png');
  return (
    <header className={clsx('hero hero--primary', styles.heroBanner)}>
      <div className="container">
        <div className={clsx(styles.heroText, 'text--center')}>
          <Heading as="h1" className="hero__title">
            The control plane
            <br />
            for KubeVirt VMs.
          </Heading>
          <p className="hero__subtitle">
            Create, operate, and govern Kubernetes VMs end to end — CLI,
            interactive TUI, and a signed-in web console on one API.
            Templates and profiles to ship, hotplug and live migrate to run,
            console/VNC/SSH to reach the guest, RBAC and quotas to stay safe,
            audit export to prove who did what. Every action hits live
            cluster objects.
          </p>
          <div className={styles.buttons}>
            <Link
              className="button button--secondary button--lg"
              to="#quickstart">
              Try it now
            </Link>
            <Link
              className="button button--outline button--lg button--secondary"
              to="https://github.com/zyvorai/zyvor-zorvia/releases/download/desktop-v0.1.0/Zorvia-0.1.0-arm64.dmg">
              Download for Mac
            </Link>
            <Link
              className="button button--outline button--lg button--secondary"
              to="/docs/LAB">
              Read the docs
            </Link>
            <Link
              className="button button--outline button--lg button--secondary"
              to="https://github.com/zyvorai/zyvor-zorvia">
              View on GitHub
            </Link>
          </div>
        </div>
      </div>
      <div className={styles.heroMediaWrap}>
        <img
          className={styles.heroMedia}
          src={dashboard}
          alt="Zorvia dashboard"
        />
        <p className={styles.heroMediaCaption}>
          The Zorvia web console — CLI, TUI, and web all driving the same
          API.
        </p>
      </div>
    </header>
  );
}

function DownloadSection() {
  return (
    <section className={styles.problem}>
      <div className="container text--center">
        <Heading as="h2" className={styles.sectionHeading}>
          Download for Mac
        </Heading>
        <p>
          A signed, notarized app with the Zorvia server bundled. Apple
          Silicon, macOS 12 or later. Point it at your kubeconfig in Settings.
        </p>
        <div className={styles.buttons}>
          <Link
            className="button button--primary button--lg"
            to="https://github.com/zyvorai/zyvor-zorvia/releases/download/desktop-v0.1.0/Zorvia-0.1.0-arm64.dmg">
            Download Zorvia 0.1.0 (.dmg)
          </Link>
          <Link
            className="button button--outline button--lg button--primary"
            to="https://github.com/zyvorai/zyvor-zorvia/releases/tag/desktop-v0.1.0">
            Release notes and checksums
          </Link>
        </div>
      </div>
    </section>
  );
}

function DemoSection() {
  const demo = useBaseUrl('/readme-demo.gif');
  return (
    <section className={styles.problem}>
      <div className="container text--center">
        <Heading as="h2" className={styles.sectionHeading}>
          See it run
        </Heading>
        <img
          className={styles.heroMedia}
          src={demo}
          alt="Animated demo: creating and operating a VM in the Zorvia console"
          loading="lazy"
        />
      </div>
    </section>
  );
}

function ProblemStatement() {
  return (
    <section className={styles.problem}>
      <div className="container">
        <div className="row">
          <div className="col col--8 col--offset-2 text--center">
            <Heading as="h2" className={styles.sectionHeading}>
              KubeVirt runs the VMs. Zorvia runs the platform.
            </Heading>
            <p>
              KubeVirt gives you the primitives — VirtualMachine,
              VirtualMachineInstance, DataVolume. Zorvia gives you the
              operator surface: shared auth, day-2 lifecycle, guest access,
              guardrails, and audit on top of those same objects.
            </p>
            <p>
              One real API behind the CLI, the interactive TUI, and the web
              console — reading and writing live cluster state, not a
              parallel store that can drift from the cluster.
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}

function TrustBand() {
  return (
    <section className={styles.trust}>
      <div className="container">
        <div className={styles.trustGrid}>
          <div>
            <Heading as="h3" className={styles.sectionHeading}>
              Apache-2.0, KubeVirt-native
            </Heading>
            <p>
              Apache-2.0 licensed, written in Rust. Zorvia doesn't
              reimplement VM execution — it's a real backend and UX layer
              on top of KubeVirt's own CRDs, so what you see in the CLI,
              TUI, or web console is what's actually running in the
              cluster.
            </p>
            <Link to="https://github.com/zyvorai/zyvor-zorvia#readme">
              Read the full README →
            </Link>
            <p>
              Coming from OpenShift Virtualization?{' '}
              <Link to="/docs/leave-openshift">Read the honest comparison →</Link>
              {' · '}
              <Link to="mailto:sales@zyvor.dev">Get Zorvia Enterprise →</Link>
            </p>
          </div>
          <div className={styles.trustBadges}>
            <img
              src="https://github.com/zyvorai/zyvor-zorvia/actions/workflows/ci.yml/badge.svg"
              alt="CI status"
            />
            <img
              src="https://img.shields.io/badge/License-Apache%202.0-blue.svg"
              alt="Apache 2.0 license"
            />
          </div>
        </div>
      </div>
    </section>
  );
}

function EnterpriseCTA() {
  return (
    <section className={styles.enterprise}>
      <div className="container text--center">
        <Heading as="h2" className={styles.sectionHeading}>
          Want to talk it through?
        </Heading>
        <p className={styles.enterpriseCopy}>
          Zorvia is Apache-2.0 and free to run in production. Reach out if
          you want to talk through a KubeVirt rollout or what Zorvia looks
          like at fleet scale.
        </p>
        <Link
          className="button button--primary button--lg"
          to="mailto:sales@zyvor.dev">
          Talk to sales
        </Link>
      </div>
    </section>
  );
}

export default function Home(): ReactNode {
  return (
    <Layout
      title="Zorvia — run Kubernetes VMs like a platform"
      description="Zorvia replaces one-off KubeVirt scripts with one real backend — CLI, interactive TUI, and a signed-in web console, all driving the same API.">
      <HomepageHeader />
      <main>
        <ProblemStatement />
        <DemoSection />
        <ProductTour />
        <FeatureHighlights />
        <DownloadSection />
        <Quickstart />
        <Migrate />
        <TrustBand />
        <EnterpriseCTA />
      </main>
    </Layout>
  );
}
