/* eslint-env node */
const path = require("path");
const HtmlWebpackPlugin = require("html-webpack-plugin");
const CopyWebpackPlugin = require("copy-webpack-plugin");
const devCerts = require("office-addin-dev-certs");

module.exports = async (env, options) => {
  const dev = options.mode === "development";
  const httpsOptions = await devCerts.getHttpsServerOptions();
  return {
    devtool: dev ? "source-map" : false,
    entry: { taskpane: "./src/taskpane/taskpane.ts" },
    output: {
      path: path.resolve(__dirname, "dist"),
      clean: true,
    },
    resolve: { extensions: [".ts", ".js", ".html"] },
    experiments: { asyncWebAssembly: true },
    module: {
      rules: [
        { test: /\.ts$/, exclude: /node_modules/, use: "ts-loader" },
        { test: /\.css$/, type: "asset/resource" },
        { test: /\.html$/, exclude: /node_modules/, use: "html-loader" },
      ],
    },
    plugins: [
      new HtmlWebpackPlugin({
        filename: "taskpane.html",
        template: "./src/taskpane/taskpane.html",
        chunks: ["taskpane"],
      }),
      new CopyWebpackPlugin({
        patterns: [
          { from: "assets", to: "assets" },
          { from: "wasm-pkg/banglakit_wasm_bg.wasm", to: "." },
        ],
      }),
    ],
    devServer: {
      server: { type: "https", options: httpsOptions },
      port: 3000,
      headers: { "Access-Control-Allow-Origin": "*" },
      static: { directory: path.join(__dirname, "dist") },
    },
  };
};
