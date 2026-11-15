import { render } from "solid-js/web";
import { App } from "./App";
import { placeOf } from "./place";
import { RepositoryPage } from "./RepositoryPage";
import "./page.css";

const root = document.getElementById("app");
const place = placeOf(location.pathname);
if (root) {
  render(() => {
    if (place.kind === "repository") return <RepositoryPage repository={place.repository} />;
    return <App shared={place.kind === "share" ? place.token : undefined} />;
  }, root);
}
