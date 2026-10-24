import NativeMaskedView from "@react-native-masked-view/masked-view";
import type { ComponentType, ReactElement } from "react";
import type { ViewProps } from "react-native";

type MaskedViewProps = ViewProps & {
  maskElement: ReactElement;
  androidRenderingMode?: "software" | "hardware";
};

/**
 * The masked view, typed against React Native's current public types.
 *
 * Its published typings (0.3.2, also the version Expo pins) build the class on
 * `NativeMethods`, which React Native 0.87 removed, so the class no longer
 * reads as a component. The runtime component is unchanged.
 */
export const MaskedView =
  NativeMaskedView as unknown as ComponentType<MaskedViewProps>;
