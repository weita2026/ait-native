. as $release
| $release.id == $config[0].release.github_release_id
  and $release.tag_name == $config[0].release.tag
  and $release.draft == false
  and $release.prerelease == true
  and all(
    $config[0].addons[];
    . as $addon
    | any(
      $release.assets[];
      .id == $addon.source_github_release_asset_id
        and .name == $addon.source_filename
        and .size == $addon.source_size_bytes
        and .digest == ("sha256:" + $addon.source_sha256)
    )
  )
