import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App.tsx";
import "./index.css";
import { AxiomProvider } from "atmx-react";
import { AxiomDefaultConfig } from "./generated/sdk.ts";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <AxiomProvider config={AxiomDefaultConfig}>
      <App />
    </AxiomProvider>
  </React.StrictMode>,
);
