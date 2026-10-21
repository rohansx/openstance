import { useState } from 'react';
import { App } from './App';
import { AppProvider } from './app/context';
import { takeTokenFromLocation } from './api/token';

export function Root() {
  // Runs once, before the router reads the hash.
  const [token] = useState(() => takeTokenFromLocation());
  return (
    <AppProvider initialToken={token}>
      <App />
    </AppProvider>
  );
}
