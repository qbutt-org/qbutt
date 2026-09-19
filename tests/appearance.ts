export function labAppearanceSettings(mode = process.env.QBUTT_LAB_APPEARANCE ?? "functional"): string[] {
    if (mode !== "functional" && mode !== "product")
        throw new Error("QBUTT_LAB_APPEARANCE must be functional or product");

    return [
        "[Preferences]", "General\\UseCustomUITheme=false",
        ...(mode === "functional" ? ["[Appearance]", "Style=Fusion", "ColorScheme=Light"] : []),
    ];
}
