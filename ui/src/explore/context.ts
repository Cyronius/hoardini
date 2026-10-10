import { createContext, useContext } from "react";

// The shell provides this once Explore is mounted; without it, nothing offers to open Explore.
export type ExploreLink = { onOpenExplore?: (path: string) => void };

export const ExploreLinkContext = createContext<ExploreLink>({});

export const useExploreLink = () => useContext(ExploreLinkContext);
