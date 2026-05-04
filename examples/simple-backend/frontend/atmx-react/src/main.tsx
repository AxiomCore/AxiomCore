import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { AxiomProvider } from "atmx-react";
import { AxiomDefaultConfig } from "./generated/sdk";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <AxiomProvider config={AxiomDefaultConfig}>
      <App />
    </AxiomProvider>
  </React.StrictMode>,
);
