import { useApp } from './app/context';
import { useRoute } from './app/router';
import { ConnectGate } from './components/ConnectGate';
import { Shell } from './components/Shell';
import { Toast } from './components/Toast';
import { Agents } from './pages/Agents';
import { Discover } from './pages/Discover';
import { EgressLog } from './pages/EgressLog';
import { Integrations } from './pages/Integrations';
import { Intros } from './pages/Intros';
import { PersonPage } from './pages/PersonPage';
import { Playbooks } from './pages/Playbooks';
import { Review } from './pages/Review';
import { Settings } from './pages/Settings';
import { Shortlist } from './pages/Shortlist';
import { Sources } from './pages/Sources';
import { Views } from './pages/Views';

export function App() {
  const { token } = useApp();
  const route = useRoute();
  if (!token) {
    return (
      <>
        <ConnectGate />
        <Toast />
      </>
    );
  }
  let page;
  switch (route.name) {
    case 'discover':
      page = <Discover mode="discover" />;
      break;
    case 'people':
      page = <Discover mode="people" />;
      break;
    case 'person':
      page = <PersonPage key={route.id} id={route.id} />;
      break;
    case 'views':
      page = <Views />;
      break;
    case 'playbooks':
      page = <Playbooks key={route.playbook ?? ''} selected={route.playbook} params={route.params ?? {}} />;
      break;
    case 'shortlist':
      page = <Shortlist />;
      break;
    case 'intros':
      page = <Intros />;
      break;
    case 'sources':
      page = <Sources />;
      break;
    case 'review':
      page = <Review />;
      break;
    case 'integrations':
      page = <Integrations />;
      break;
    case 'agents':
      page = <Agents />;
      break;
    case 'egress':
      page = <EgressLog />;
      break;
    case 'settings':
      page = <Settings />;
      break;
  }
  return (
    <>
      <a className="skip" href="#main" onClick={(e) => { e.preventDefault(); document.getElementById('main')?.focus(); }}>
        Skip to workspace
      </a>
      <Shell route={route}>{page}</Shell>
      <Toast />
    </>
  );
}
