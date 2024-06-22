/** @type {import('tailwindcss').Config} */
module.exports = {
  content: {
    relative: true,
    files: ["*.html", "./src/**/*.rs"],
  },
  theme: {
    extend: {
      backgroundImage: {
        "muni-bot": "url('/muni_bot_bg.jpg')",
      },
    },
  },
  plugins: [],
};
