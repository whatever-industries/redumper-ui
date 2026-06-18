import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: "#15181d",
        panel: "#f7f7f3",
        rail: "#252a31",
        copper: "#b65735",
        mint: "#0066CC"
      },
      boxShadow: {
        focus: "0 0 0 3px rgba(0, 102, 204, 0.25)"
      }
    }
  },
  plugins: []
} satisfies Config;
