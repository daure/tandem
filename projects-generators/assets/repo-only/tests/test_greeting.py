import unittest

from greeting import greet


class GreetingTests(unittest.TestCase):
    def test_names_are_trimmed_and_empty_names_use_world(self):
        self.assertEqual(greet(" Tandem "), "Hello, Tandem!")
        self.assertEqual(greet(" "), "Hello, world!")
