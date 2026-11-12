import { mount } from 'svelte';
import './tokens.css';
import App from './App.svelte';
import { registerServiceWorker } from './lib/pwa.svelte';

mount(App, { target: document.getElementById('app')! });
registerServiceWorker();
