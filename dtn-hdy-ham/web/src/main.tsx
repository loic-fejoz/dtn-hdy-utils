import { render } from 'preact';
import { App } from './App';
import './index.css';
import { registerServiceWorker } from './utils/pwa';

registerServiceWorker();

const root = document.getElementById('app');
if (root) {
  render(<App />, root);
}
