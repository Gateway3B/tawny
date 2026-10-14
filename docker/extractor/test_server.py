import unittest

import server


class CommandTests(unittest.TestCase):
    def test_command_has_token_provider_and_supported_js_runtime(self) -> None:
        command = server.build_video_command("aNXB-8Aqt88")
        self.assertIn("node", command)
        # Pinning a player client is what broke extraction once already: `mweb`
        # started returning "Video unavailable" and playback fell back to gated
        # URLs that die after about a minute. Let yt-dlp choose.
        self.assertFalse(
            any(value.startswith("youtube:player_client=") for value in command)
        )
        self.assertTrue(
            any(value.startswith("youtubepot-bgutilhttp:base_url=") for value in command)
        )

    def test_command_rejects_non_video_ids(self) -> None:
        with self.assertRaises(ValueError):
            server.build_video_command("../../secrets")

    def test_shorts_command_is_flat_and_bounded(self) -> None:
        command = server.build_channel_shorts_command("UCsXVk37bltHxD1rDPwtNM8Q")
        self.assertIn("--flat-playlist", command)
        self.assertIn("30", command)
        self.assertTrue(command[-1].endswith("/shorts"))

    def test_shorts_command_rejects_non_channel_ids(self) -> None:
        with self.assertRaises(ValueError):
            server.build_channel_shorts_command("veritasium")

    def test_upload_listing_keeps_dates_and_covers_the_complete_tab(self) -> None:
        command = server.build_channel_uploads_command("UCsXVk37bltHxD1rDPwtNM8Q", "videos")
        self.assertIn("--flat-playlist", command)
        self.assertIn("youtubetab:approximate_date", command)
        self.assertNotIn("--playlist-end", command)
        self.assertTrue(command[-1].endswith("/videos"))

    def test_upload_listing_rejects_unknown_tabs_and_channel_paths(self) -> None:
        for channel, tab in [("../../secret", "videos"), ("UCsXVk37bltHxD1rDPwtNM8Q", "../videos")]:
            with self.assertRaises(ValueError):
                server.build_channel_uploads_command(channel, tab)


if __name__ == "__main__":
    unittest.main()
