import React from 'react';
import ReactDOM from 'react-dom/client';
import '@xyflow/react/dist/style.css';
import './styles.css';
import { SchemaApp } from './schema-app';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <SchemaApp />
  </React.StrictMode>,
);
