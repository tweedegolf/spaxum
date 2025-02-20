import React from 'react';
import { createRoot } from 'react-dom/client';
import './index.css';

function HelloMessage({ name }) {
  return <div>Hello {name}</div>;
}

const root = createRoot(document.getElementById('root'));
root.render(<HelloMessage name="World" />);
