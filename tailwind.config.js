/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./*.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: {
          950: "#0e0d0b",
          900: "#161410",
          800: "#1f1c16",
          700: "#2a261e",
          500: "#6f6759",
          300: "#c9c0b0",
          100: "#f4efe6",
        },
        ember: {
          400: "#f5a524",
          500: "#e08b12",
          600: "#c46f0a",
        },
        ice: {
          400: "#8fd3e8",
          500: "#5aa7c4",
        },
      },
      fontFamily: {
        display: ["Fraunces", "Georgia", "serif"],
        sans: ["IBM Plex Sans", "ui-sans-serif", "system-ui", "sans-serif"],
        mono: ["IBM Plex Mono", "ui-monospace", "monospace"],
      },
      boxShadow: {
        tray: "0 18px 50px rgba(0,0,0,0.45)",
      },
    },
  },
  plugins: [],
};
