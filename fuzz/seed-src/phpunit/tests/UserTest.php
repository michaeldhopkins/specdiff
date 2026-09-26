<?php
class BaseTestCase extends TestCase {
    public function testShared(): void {
        $this->assertTrue(true);
    }
}

class UserTest extends BaseTestCase {
    public function testCreate(): void {
        $this->assertTrue(true);
    }

    /**
     * @dataProvider emails
     */
    public function testEmail(string $email): void {
    }

    public function helperMethod(): void {
    }
}
