import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { Root } from './Root';
import './styles/prototype-base.css';
import './styles/prototype-workspace.css';
import './styles/prototype-integrations.css';
import './styles/app.css';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <Root />
  </StrictMode>,
);
