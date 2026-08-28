import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App.tsx';
import './app.css';

const root = document.getElementById('root');
if (!root) throw new Error('dashboard.html is missing its #root element');

createRoot(root).render(
    <StrictMode>
        <App />
    </StrictMode>,
);
