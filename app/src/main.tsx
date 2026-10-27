import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { handleBackButton } from "./back";
import { I18nProvider } from "./i18n";
import { applyTheme } from "./theme";
import "@fontsource-variable/figtree";
import "./styles.css";

applyTheme();
handleBackButton();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <I18nProvider>
      <App />
    </I18nProvider>
  </StrictMode>,
);
