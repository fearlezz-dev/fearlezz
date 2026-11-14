import { render } from "solid-js/web";
import { App } from "./App";
import "./page.css";

const root = document.getElementById("app");
if (root) render(() => <App />, root);
