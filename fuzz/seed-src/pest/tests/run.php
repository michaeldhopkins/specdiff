<?php

uses(TestCase::class)->in('Feature');
beforeEach(function () {});

describe('User', function () {
    test('is created', function () {});
    it('can be deleted', function () {});
});

test("user $name is created", function () {});
test('validates email', function ($email) {})->with(['a@b.com', 'c@d.com', 'e@f.com']);
test('z', function () {})->group('api')->skip();
