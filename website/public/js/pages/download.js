// Point each download button at the matching file in the latest GitHub release.
// Release file names carry the version, so they're looked up rather than hard-coded.
// If the API can't be reached the buttons keep linking to the Releases page.
(function () {
  var api = 'https://api.github.com/repos/jasonsutter87/writepad/releases/latest';

  fetch(api, { headers: { Accept: 'application/vnd.github+json' } })
    .then(function (res) { return res.ok ? res.json() : Promise.reject(res.status); })
    .then(function (release) {
      document.querySelectorAll('[data-asset]').forEach(function (link) {
        var suffix = link.getAttribute('data-asset');
        var asset = release.assets.find(function (a) { return a.name.endsWith(suffix); });
        if (asset) link.href = asset.browser_download_url;
      });
      var version = (release.tag_name || '').replace(/^v/, '');
      document.querySelectorAll('[data-release-version]').forEach(function (el) {
        if (version) el.textContent = version;
      });
    })
    .catch(function () {});
})();
